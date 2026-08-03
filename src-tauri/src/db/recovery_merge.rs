use super::{quote_identifier, table_exists};
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, Row, TransactionBehavior,
};

pub(super) const STATE_RECOVERY_TABLES: &[&str] = &[
    "intents",
    "actions",
    "certificates",
    "plan_generation_states",
    "agent_execution_logs",
    "agent_executions",
    "chat_messages",
    "chat_sessions",
    "chat_turns",
    "verified_filesystem_contexts",
    "pending_contextual_file_actions",
    "workflows",
    "workflow_approvals",
    "routing_preferences",
    "app_preferences",
    "user_routing_preferences",
    "active_session_configs",
    "message_queue",
    "gateway_message_receipts",
    "workflow_blueprints",
    "compiled_instructions",
    "execution_instances",
    "workflow_schedules",
    "sovereign_trust_policies",
    "active_trust_sessions",
];

pub(super) const OPERATIONS_RECOVERY_TABLES: &[&str] = &["local_inference_audit"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RecoveryMergeAssessment {
    pub source_records: usize,
    pub new_records: usize,
    pub identical_records: usize,
    pub source_newer_records: usize,
    pub durable_newer_records: usize,
    pub conflicting_records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableShape {
    columns: Vec<String>,
    primary_key_indexes: Vec<usize>,
}

pub(super) fn assess_recovery_records(
    source: &Connection,
    destination: &Connection,
    tables: &[&str],
) -> Result<RecoveryMergeAssessment, String> {
    let mut assessment = RecoveryMergeAssessment::default();
    for table in tables {
        let conflicts_before = assessment.conflicting_records;
        if !table_exists(source, table).map_err(|error| error.to_string())? {
            continue;
        }
        if !table_exists(destination, table).map_err(|error| error.to_string())? {
            return Err(format!(
                "Recovery destination is missing required table {}.",
                table
            ));
        }
        let source_shape = table_shape(source, table)?;
        let destination_shape = table_shape(destination, table)?;
        if source_shape != destination_shape {
            return Err(format!(
                "Recovery table {} does not match the durable schema.",
                table
            ));
        }

        for values in read_table_rows(source, table, &source_shape)? {
            assessment.source_records = assessment.source_records.saturating_add(1);
            let durable = find_row(destination, table, &source_shape, &values)?;
            match durable {
                None => assessment.new_records = assessment.new_records.saturating_add(1),
                Some(durable) if durable == values => {
                    assessment.identical_records = assessment.identical_records.saturating_add(1)
                }
                Some(durable) => match newer_record(table, &source_shape, &values, &durable) {
                    Some(true) => {
                        assessment.source_newer_records =
                            assessment.source_newer_records.saturating_add(1)
                    }
                    Some(false) => {
                        assessment.durable_newer_records =
                            assessment.durable_newer_records.saturating_add(1)
                    }
                    None => {
                        assessment.conflicting_records =
                            assessment.conflicting_records.saturating_add(1)
                    }
                },
            }
        }
        let table_conflicts = assessment
            .conflicting_records
            .saturating_sub(conflicts_before);
        if table_conflicts != 0 {
            eprintln!(
                "OOMU_RECOVERY_RECORD_CONFLICT table={} records={}",
                table, table_conflicts
            );
        }
    }
    Ok(assessment)
}

pub(super) fn merge_non_conflicting_recovery_records(
    source: &Connection,
    destination: &mut Connection,
    tables: &[&str],
) -> Result<RecoveryMergeAssessment, String> {
    let assessment = assess_recovery_records(source, destination, tables)?;
    if assessment.conflicting_records != 0 {
        return Err(
            "Recovery merge refused records with genuinely different saved values.".to_string(),
        );
    }
    if assessment.new_records == 0 && assessment.source_newer_records == 0 {
        return Ok(assessment);
    }

    let transaction = destination
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    transaction
        .execute_batch("PRAGMA defer_foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    let mut inserted = 0usize;
    let mut updated = 0usize;

    for table in tables {
        if !table_exists(source, table).map_err(|error| error.to_string())? {
            continue;
        }
        let shape = table_shape(source, table)?;
        let columns = shape
            .columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=shape.columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!(
            "INSERT INTO {} ({columns}) VALUES ({placeholders})",
            quote_identifier(table)
        );

        for values in read_table_rows(source, table, &shape)? {
            match find_row(&transaction, table, &shape, &values)? {
                Some(durable) if durable == values => continue,
                Some(durable) => match newer_record(table, &shape, &values, &durable) {
                    Some(true) => {
                        update_row(&transaction, table, &shape, &values)?;
                        updated = updated.saturating_add(1);
                        continue;
                    }
                    Some(false) => continue,
                    None => {
                        return Err(format!(
                            "Recovery record in {} changed after the safety comparison.",
                            table
                        ));
                    }
                },
                None => {}
            }
            transaction
                .execute(&insert_sql, params_from_iter(values.iter()))
                .map_err(|error| {
                    format!(
                        "Recovery could not safely add a record to {}: {}",
                        table, error
                    )
                })?;
            let persisted = find_row(&transaction, table, &shape, &values)?;
            if persisted.as_ref() != Some(&values) {
                return Err(format!(
                    "Recovery verification failed after adding a record to {}.",
                    table
                ));
            }
            inserted = inserted.saturating_add(1);
        }
    }

    let foreign_key_failures: i64 = transaction
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if foreign_key_failures != 0 {
        return Err(format!(
            "Recovery merge would leave {foreign_key_failures} invalid relationship(s)."
        ));
    }
    if inserted != assessment.new_records {
        return Err("Recovery merge inserted an unexpected number of records.".to_string());
    }
    if updated != assessment.source_newer_records {
        return Err("Recovery merge updated an unexpected number of records.".to_string());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(assessment)
}

fn table_shape(connection: &Connection, table: &str) -> Result<TableShape, String> {
    let mut statement = connection
        .prepare("SELECT name, pk FROM pragma_table_info(?1) ORDER BY cid")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut described_columns = Vec::new();
    for row in rows {
        let (name, primary_key_order) = row.map_err(|error| error.to_string())?;
        described_columns.push((name, primary_key_order));
    }
    if described_columns.is_empty() {
        return Err(format!("Recovery table {} has no readable columns.", table));
    }
    // SQLite preserves the physical column order from the database's original
    // migration history. A newly created database and an older database that
    // reached the same schema through ALTER TABLE can therefore expose the
    // same named columns in different orders. Recovery is bound to column
    // names, not incidental storage order.
    described_columns.sort_by(|left, right| left.0.cmp(&right.0));
    let columns = described_columns
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let mut primary_keys = described_columns
        .iter()
        .enumerate()
        .filter_map(|(index, (_, order))| (*order != 0).then_some((*order, index)))
        .collect::<Vec<_>>();
    primary_keys.sort_by_key(|(order, _)| *order);
    if primary_keys.is_empty() {
        return Err(format!(
            "Recovery table {} has no stable primary key.",
            table
        ));
    }
    Ok(TableShape {
        columns,
        primary_key_indexes: primary_keys.into_iter().map(|(_, index)| index).collect(),
    })
}

fn read_table_rows(
    connection: &Connection,
    table: &str,
    shape: &TableShape,
) -> Result<Vec<Vec<Value>>, String> {
    let columns = shape
        .columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {columns} FROM {}", quote_identifier(table));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| read_values(row, shape.columns.len()))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn find_row(
    connection: &Connection,
    table: &str,
    shape: &TableShape,
    source_values: &[Value],
) -> Result<Option<Vec<Value>>, String> {
    let columns = shape
        .columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = shape
        .primary_key_indexes
        .iter()
        .enumerate()
        .map(|(parameter, index)| {
            format!(
                "{} IS ?{}",
                quote_identifier(&shape.columns[*index]),
                parameter + 1
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let primary_key = shape
        .primary_key_indexes
        .iter()
        .map(|index| &source_values[*index]);
    let sql = format!(
        "SELECT {columns} FROM {} WHERE {predicate} LIMIT 1",
        quote_identifier(table)
    );
    connection
        .query_row(&sql, params_from_iter(primary_key), |row| {
            read_values(row, shape.columns.len())
        })
        .optional()
        .map_err(|error| error.to_string())
}

fn read_values(row: &Row<'_>, count: usize) -> rusqlite::Result<Vec<Value>> {
    (0..count).map(|index| row.get(index)).collect()
}

fn newer_record(
    table: &str,
    shape: &TableShape,
    source: &[Value],
    durable: &[Value],
) -> Option<bool> {
    let revision_column = match table {
        "agent_executions" | "chat_sessions" | "active_session_configs" => "updated_at_ms",
        "workflows" | "routing_preferences" => "updated_at",
        "app_preferences" | "user_routing_preferences" => "updated_at_ms",
        "workflow_blueprints" | "execution_instances" | "workflow_schedules" => "updated_at_ms",
        "active_trust_sessions" => "last_activity_at_ms",
        _ => return None,
    };
    let index = shape
        .columns
        .iter()
        .position(|column| column == revision_column)?;
    let Value::Integer(source_revision) = source.get(index)? else {
        return None;
    };
    let Value::Integer(durable_revision) = durable.get(index)? else {
        return None;
    };
    match source_revision.cmp(durable_revision) {
        std::cmp::Ordering::Greater => Some(true),
        std::cmp::Ordering::Less => Some(false),
        std::cmp::Ordering::Equal => None,
    }
}

fn update_row(
    connection: &Connection,
    table: &str,
    shape: &TableShape,
    values: &[Value],
) -> Result<(), String> {
    let mutable_indexes = (0..shape.columns.len())
        .filter(|index| !shape.primary_key_indexes.contains(index))
        .collect::<Vec<_>>();
    let assignments = mutable_indexes
        .iter()
        .enumerate()
        .map(|(parameter, index)| {
            format!(
                "{} = ?{}",
                quote_identifier(&shape.columns[*index]),
                parameter + 1
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = shape
        .primary_key_indexes
        .iter()
        .enumerate()
        .map(|(parameter, index)| {
            format!(
                "{} IS ?{}",
                quote_identifier(&shape.columns[*index]),
                mutable_indexes.len() + parameter + 1
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let parameters = mutable_indexes
        .iter()
        .chain(shape.primary_key_indexes.iter())
        .map(|index| &values[*index]);
    let sql = format!(
        "UPDATE {} SET {assignments} WHERE {predicate}",
        quote_identifier(table)
    );
    let changed = connection
        .execute(&sql, params_from_iter(parameters))
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(format!(
            "Recovery expected to update exactly one record in {}.",
            table
        ));
    }
    let persisted = find_row(connection, table, shape, values)?;
    if persisted.as_deref() != Some(values) {
        return Err(format!(
            "Recovery could not verify the updated record in {}.",
            table
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE records (id TEXT PRIMARY KEY, value TEXT NOT NULL, updated INTEGER NOT NULL);",
            )
            .unwrap();
        connection
    }

    fn preference_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE app_preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL, encryption_state TEXT NOT NULL);",
            )
            .unwrap();
        connection
    }

    #[test]
    fn disjoint_records_merge_without_replacing_durable_rows() {
        let source = connection();
        let mut durable = connection();
        source
            .execute("INSERT INTO records VALUES ('source', 'new', 2)", [])
            .unwrap();
        durable
            .execute("INSERT INTO records VALUES ('durable', 'keep', 1)", [])
            .unwrap();

        let report =
            merge_non_conflicting_recovery_records(&source, &mut durable, &["records"]).unwrap();

        assert_eq!(report.new_records, 1);
        assert_eq!(report.conflicting_records, 0);
        let rows: i64 = durable
            .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn equivalent_columns_merge_across_different_physical_orders() {
        let source = Connection::open_in_memory().unwrap();
        let mut durable = Connection::open_in_memory().unwrap();
        source
            .execute_batch(
                "CREATE TABLE records (id TEXT PRIMARY KEY, value TEXT NOT NULL, updated INTEGER NOT NULL);\
                 INSERT INTO records VALUES ('source', 'new', 2);",
            )
            .unwrap();
        durable
            .execute_batch(
                "CREATE TABLE records (updated INTEGER NOT NULL, id TEXT PRIMARY KEY, value TEXT NOT NULL);\
                 INSERT INTO records VALUES (1, 'durable', 'keep');",
            )
            .unwrap();

        let report =
            merge_non_conflicting_recovery_records(&source, &mut durable, &["records"]).unwrap();

        assert_eq!(report.new_records, 1);
        let values = durable
            .prepare("SELECT id, value FROM records ORDER BY id")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            values,
            vec![
                ("durable".to_string(), "keep".to_string()),
                ("source".to_string(), "new".to_string())
            ]
        );
    }

    #[test]
    fn identical_records_are_idempotently_skipped() {
        let source = connection();
        let mut durable = connection();
        source
            .execute("INSERT INTO records VALUES ('same', 'value', 1)", [])
            .unwrap();
        durable
            .execute("INSERT INTO records VALUES ('same', 'value', 1)", [])
            .unwrap();

        let report =
            merge_non_conflicting_recovery_records(&source, &mut durable, &["records"]).unwrap();

        assert_eq!(report.identical_records, 1);
        assert_eq!(report.new_records, 0);
        assert_eq!(report.conflicting_records, 0);
    }

    #[test]
    fn genuinely_different_values_require_confirmation() {
        let source = connection();
        let durable = connection();
        source
            .execute("INSERT INTO records VALUES ('same', 'recovery', 2)", [])
            .unwrap();
        durable
            .execute("INSERT INTO records VALUES ('same', 'durable', 3)", [])
            .unwrap();

        let report = assess_recovery_records(&source, &durable, &["records"]).unwrap();

        assert_eq!(report.conflicting_records, 1);
        assert_eq!(report.new_records, 0);
        assert_eq!(report.identical_records, 0);
    }

    #[test]
    fn newer_preferences_replace_only_the_matching_durable_value() {
        let source = preference_connection();
        let mut durable = preference_connection();
        source
            .execute(
                "INSERT INTO app_preferences VALUES ('theme', 'dark', 20, 'encrypted')",
                [],
            )
            .unwrap();
        durable
            .execute(
                "INSERT INTO app_preferences VALUES ('theme', 'light', 10, 'encrypted')",
                [],
            )
            .unwrap();

        let report =
            merge_non_conflicting_recovery_records(&source, &mut durable, &["app_preferences"])
                .unwrap();

        assert_eq!(report.source_newer_records, 1);
        let value: String = durable
            .query_row(
                "SELECT value FROM app_preferences WHERE key='theme'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "dark");
    }

    #[test]
    fn newer_durable_preferences_are_preserved() {
        let source = preference_connection();
        let mut durable = preference_connection();
        source
            .execute(
                "INSERT INTO app_preferences VALUES ('theme', 'light', 10, 'encrypted')",
                [],
            )
            .unwrap();
        durable
            .execute(
                "INSERT INTO app_preferences VALUES ('theme', 'dark', 20, 'encrypted')",
                [],
            )
            .unwrap();

        let report =
            merge_non_conflicting_recovery_records(&source, &mut durable, &["app_preferences"])
                .unwrap();

        assert_eq!(report.durable_newer_records, 1);
        let value: String = durable
            .query_row(
                "SELECT value FROM app_preferences WHERE key='theme'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "dark");
    }
}
