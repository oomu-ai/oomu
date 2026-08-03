use super::{MemoryLedger, MemoryLedgerError};
use rusqlite::params;

impl MemoryLedger {
    pub(crate) fn purge_project(&self, raw_project_id: &str) -> Result<usize, MemoryLedgerError> {
        let project_id = crate::p0_contracts::ProjectId::parse(raw_project_id)
            .map_err(|error| MemoryLedgerError::invalid(&error))?
            .to_string();
        let _guard = self.lock_writes();
        self.open_connection()
            .map_err(MemoryLedgerError::database)?
            .execute(
                "DELETE FROM agent_memory_entries WHERE project_id=?1",
                params![project_id],
            )
            .map_err(MemoryLedgerError::database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn purge_project_removes_only_that_projects_agent_memory() {
        let root = std::env::temp_dir().join(format!(
            "oomu-project-memory-purge-{}",
            crate::p0_contracts::ProjectId::new()
        ));
        fs::create_dir_all(&root).unwrap();
        let ledger = MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
        let first = crate::p0_contracts::ProjectId::new().to_string();
        let second = crate::p0_contracts::ProjectId::new().to_string();
        let connection = ledger.open_connection().unwrap();
        for (index, project_id) in [&first, &second].into_iter().enumerate() {
            connection.execute(
                "INSERT INTO agent_memory_entries(agent_id,memory_kind,scope,project_id,content,confidence,source_session,visibility,signature_json,created_at_ms) VALUES ('agent','project_fact',?1,?2,?3,1.0,'session','private','{}',1)",
                params![format!("project:{project_id}:fact"), project_id, format!("fact-{index}")],
            ).unwrap();
        }
        drop(connection);

        assert_eq!(ledger.purge_project(&first).unwrap(), 1);
        let connection = ledger.open_connection().unwrap();
        let first_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_memory_entries WHERE project_id=?1",
                params![first],
                |row| row.get(0),
            )
            .unwrap();
        let second_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_memory_entries WHERE project_id=?1",
                params![second],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(first_count, 0);
        assert_eq!(second_count, 1);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
}
