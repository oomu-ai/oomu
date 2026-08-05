use super::*;

pub(super) fn accepts_verified_legacy_checksum(
    checksum: &str,
    migration: MigrationDescriptor,
) -> bool {
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

pub(super) fn verify_agent_execution_origin_index(connection: &Connection) -> rusqlite::Result<()> {
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
