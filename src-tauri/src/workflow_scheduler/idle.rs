use super::*;

pub(super) fn scheduler_requires_polling(
    persistence: &PersistenceEngine,
) -> rusqlite::Result<bool> {
    let connection = persistence.open_connection()?;
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM workflow_schedules WHERE is_active = 1
            UNION ALL
            SELECT 1 FROM routine_delivery_receipts WHERE state IN ('pending', 'failed')
            LIMIT 1
        )",
        [],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scheduler_can_sleep_until_work_is_announced() {
        let root = std::env::temp_dir().join(format!(
            "oomu-scheduler-idle-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        assert!(!scheduler_requires_polling(&engine).unwrap());

        let connection = engine.open_connection().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=OFF;
                 INSERT INTO workflow_schedules (
                    id, workflow_id, label, schedule_expression, run_request_json,
                    is_active, next_run_at_ms, created_at_ms, updated_at_ms
                 ) VALUES (
                    'scheduled-work', 'workflow', 'Scheduled work', 'every 1 hour',
                    '{}', 1, 9223372036854775807, 1, 1
                 );",
            )
            .unwrap();
        drop(connection);
        assert!(scheduler_requires_polling(&engine).unwrap());
        drop(engine);
        std::fs::remove_dir_all(root).unwrap();
    }
}
