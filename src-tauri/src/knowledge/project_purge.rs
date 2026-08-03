use super::{KnowledgeError, KnowledgeStore};
use rusqlite::params;

impl KnowledgeStore {
    pub(crate) fn purge_project(&self, raw_project_id: &str) -> Result<usize, KnowledgeError> {
        let project_id = crate::p0_contracts::ProjectId::parse(raw_project_id)
            .map_err(KnowledgeError::invalid)?
            .to_string();
        let _guard = self.lock_writes();
        let mut connection = self.open_connection().map_err(KnowledgeError::database)?;
        let transaction = connection.transaction().map_err(KnowledgeError::database)?;
        let chunk_count = transaction
            .execute(
                "DELETE FROM knowledge_chunks WHERE project_id=?1",
                params![project_id],
            )
            .map_err(KnowledgeError::database)?;
        let document_count = transaction
            .execute(
                "DELETE FROM knowledge_documents WHERE project_id=?1",
                params![project_id],
            )
            .map_err(KnowledgeError::database)?;
        transaction.commit().map_err(KnowledgeError::database)?;
        Ok(chunk_count.saturating_add(document_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{foundation::clock::unix_time_ms_i64, p0_contracts::ProjectId};
    use std::fs;

    #[test]
    fn purge_project_removes_only_that_projects_private_index() {
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-knowledge-project-purge-{}",
            unix_time_ms_i64()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let store = KnowledgeStore::initialize_at(temp_dir.join("knowledge.db")).unwrap();
        let first = ProjectId::new().to_string();
        let second = ProjectId::new().to_string();
        let connection = store.open_connection().unwrap();
        for (index, project_id) in [&first, &second].into_iter().enumerate() {
            let path = format!("projects/{project_id}/notes-{index}.md");
            connection.execute(
                "INSERT INTO knowledge_documents(path,workspace_id,mod_id,workspace_root,project_id,content_hash,modified_ms,ingested_ms,chunk_count) VALUES (?1,'workspace','project','/private',?2,'hash',1,1,1)",
                params![path, project_id],
            ).unwrap();
            connection.execute(
                "INSERT INTO knowledge_chunks(path,workspace_id,mod_id,workspace_root,project_id,chunk_index,line_start,line_end,snippet,embedding_json,embedding_source) VALUES (?1,'workspace','project','/private',?2,0,1,1,'private note','[]','test')",
                params![path, project_id],
            ).unwrap();
        }
        drop(connection);

        assert_eq!(store.purge_project(&first).unwrap(), 2);
        let connection = store.open_connection().unwrap();
        let first_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM knowledge_documents WHERE project_id=?1",
                params![first],
                |row| row.get(0),
            )
            .unwrap();
        let second_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM knowledge_documents WHERE project_id=?1",
                params![second],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(first_count, 0);
        assert_eq!(second_count, 1);
        drop(connection);
        let _ = fs::remove_dir_all(temp_dir);
    }
}
