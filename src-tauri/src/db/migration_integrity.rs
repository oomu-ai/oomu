use super::*;

pub(super) const ADAPTIVE_LEARNING_SQL: &str =
    include_str!("../../migrations/0023_adaptive_learning.sql");
pub(super) const ADAPTIVE_LEARNING_DESCRIPTOR: MigrationDescriptor =
    sql(23, "0023_adaptive_learning", ADAPTIVE_LEARNING_SQL);

pub(super) fn accepts_legacy_checksum(checksum: &str, migration: MigrationDescriptor) -> bool {
    let legacy_runner_checksum = matches!(migration.sequence, 1 | 3 | 4 | 6 | 7)
        && matches!(migration.source, MigrationSource::RustImplementation { .. });
    // Early beta builds recorded a different source checksum for this SQL
    // migration even though they installed the compatible adaptive-learning
    // schema. The caller must verify the complete schema contract before
    // accepting either compatibility case.
    let adaptive_learning_beta_checksum = migration.sequence == 23
        && migration.id == "0023_adaptive_learning"
        && matches!(migration.source, MigrationSource::Sql(_));
    (legacy_runner_checksum || adaptive_learning_beta_checksum)
        && checksum.len() == 64
        && checksum
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn verify_adaptive_learning_schema(connection: &Connection) -> rusqlite::Result<()> {
    require_schema_objects(
        connection,
        "table",
        &["learning_offers", "saved_methods", "saved_method_versions"],
    )?;
    require_schema_objects(
        connection,
        "index",
        &[
            "idx_learning_offers_task",
            "idx_learning_offers_project",
            "idx_saved_methods_project",
        ],
    )?;
    require_columns(
        connection,
        "learning_offers",
        &[
            "offer_id",
            "project_id",
            "task_id",
            "task_run_id",
            "kind",
            "status",
            "summary",
            "proposed_method_json",
            "source_evidence_json",
            "exposure_summary",
            "conflict_summary",
            "created_at_ms",
            "reviewed_at_ms",
        ],
    )?;
    require_columns(
        connection,
        "saved_methods",
        &[
            "method_id",
            "source_offer_id",
            "project_id",
            "name",
            "summary",
            "current_version",
            "enabled",
            "use_count",
            "successful_use_count",
            "intervention_count",
            "deleted_at_ms",
            "created_at_ms",
            "updated_at_ms",
        ],
    )?;
    require_columns(
        connection,
        "saved_method_versions",
        &[
            "method_id",
            "version",
            "method_json",
            "source_task_run_id",
            "change_summary",
            "created_at_ms",
        ],
    )?;
    Ok(())
}

pub(super) fn verify_adaptive_learning_if_applied(
    connection: &Connection,
    through: i64,
) -> rusqlite::Result<()> {
    if through < ADAPTIVE_LEARNING_DESCRIPTOR.sequence {
        return Ok(());
    }
    verify_adaptive_learning_schema(connection)
}

pub(super) fn verify_origin_index(connection: &Connection) -> rusqlite::Result<()> {
    let (is_unique, is_partial): (i64, i64) = connection.query_row(
        "SELECT \"unique\", partial FROM pragma_index_list('agent_executions')
         WHERE name='idx_agent_executions_active_plan_origin'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let indexed_columns: String = connection.query_row(
        "SELECT group_concat(name, ',') FROM (
             SELECT name FROM pragma_index_info('idx_agent_executions_active_plan_origin')
             ORDER BY seqno
         )",
        [],
        |row| row.get(0),
    )?;
    if is_unique == 1 && is_partial == 1 && indexed_columns == "plan_id,turn_id,generation_token" {
        Ok(())
    } else {
        Err(migration_recovery_error(
            "agent execution origin index does not enforce the required unique partial key",
        ))
    }
}
