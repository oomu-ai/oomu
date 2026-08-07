use super::*;
use crate::{db::PersistenceEngine, p0_contracts::ProjectId};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension, Row};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) const INTERNAL_LOCAL_FILES_PROJECT_ID: &str =
    "project_00000000-0000-4000-8000-000000000001";
const INTERNAL_LOCAL_FILES_PROJECT_NAME: &str = "My files";
const INTERNAL_LOCAL_FILES_PROJECT_DESCRIPTION: &str =
    "Private workspace used for files created from Chat.";
const INTERNAL_LOCAL_FILES_PROJECT_MANAGED_ERROR: &str =
    "This private OOMU workspace is managed automatically.";

pub(crate) fn user_managed_project_id(raw_id: &str) -> Result<String, String> {
    let project_id = ProjectId::parse(raw_id)?.to_string();
    if project_id == INTERNAL_LOCAL_FILES_PROJECT_ID {
        return Err(INTERNAL_LOCAL_FILES_PROJECT_MANAGED_ERROR.to_string());
    }
    Ok(project_id)
}

pub(crate) fn validate_user_project(
    engine: &PersistenceEngine,
    raw_id: &str,
) -> Result<String, String> {
    let project_id = user_managed_project_id(raw_id)?;
    let project = get(engine, &project_id)?;
    if project.archived_at_ms.is_some() {
        return Err("Archived Projects cannot receive new conversations.".to_string());
    }
    Ok(project_id)
}

pub(crate) fn ensure_internal_local_files_project(
    connection: &rusqlite::Connection,
) -> rusqlite::Result<String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    connection.execute(
        "INSERT INTO projects (project_id,name,description,created_at_ms,updated_at_ms,archived_at_ms) VALUES (?1,?2,?3,?4,?4,NULL)
         ON CONFLICT(project_id) DO UPDATE SET name=excluded.name,description=excluded.description,updated_at_ms=excluded.updated_at_ms,archived_at_ms=NULL",
        params![
            INTERNAL_LOCAL_FILES_PROJECT_ID,
            INTERNAL_LOCAL_FILES_PROJECT_NAME,
            INTERNAL_LOCAL_FILES_PROJECT_DESCRIPTION,
            now
        ],
    )?;
    connection.execute(
        "INSERT INTO project_policy (project_id,data_policy,updated_at_ms) VALUES (?1,'local_only',?2)
         ON CONFLICT(project_id) DO UPDATE SET data_policy='local_only',updated_at_ms=excluded.updated_at_ms",
        params![INTERNAL_LOCAL_FILES_PROJECT_ID, now],
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO project_instructions (project_id,instructions,updated_at_ms) VALUES (?1,'',?2)",
        params![INTERNAL_LOCAL_FILES_PROJECT_ID, now],
    )?;
    Ok(INTERNAL_LOCAL_FILES_PROJECT_ID.to_string())
}

fn clean_text(value: &str, field: &str, max: usize, required: bool) -> Result<String, String> {
    let value = value.trim();
    if (required && value.is_empty()) || value.chars().count() > max {
        return Err(format!("Invalid project {field}."));
    }
    Ok(value.to_string())
}

fn project_from_row(row: &Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let raw_policy: String = row.get(3)?;
    let data_policy = match raw_policy.as_str() {
        "local_only" => ProjectDataPolicy::LocalOnly,
        "allow_configured_cloud" => ProjectDataPolicy::AllowConfiguredCloud,
        _ => ProjectDataPolicy::AskBeforeCloud,
    };
    Ok(ProjectRecord {
        project_id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        data_policy,
        instructions: row.get(4)?,
        archived_at_ms: row.get(5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
        source_count: row.get::<_, i64>(8)? as usize,
        conversation_count: row.get::<_, i64>(9)? as usize,
        workflow_count: row.get::<_, i64>(10)? as usize,
        task_count: row.get::<_, i64>(11)? as usize,
    })
}

const PROJECT_SELECT: &str = "
SELECT p.project_id, p.name, p.description, policy.data_policy,
       COALESCE(instructions.instructions, ''), p.archived_at_ms, p.created_at_ms, p.updated_at_ms,
       (SELECT COUNT(*) FROM project_sources s WHERE s.project_id=p.project_id),
       (SELECT COUNT(*) FROM chat_sessions c WHERE c.project_id=p.project_id),
       (SELECT COUNT(*) FROM workflow_blueprints w WHERE w.project_id=p.project_id),
       (SELECT COUNT(*) FROM task_runs t WHERE t.project_id=p.project_id)
FROM projects p
JOIN project_policy policy ON policy.project_id=p.project_id
LEFT JOIN project_instructions instructions ON instructions.project_id=p.project_id";

pub(crate) fn create(
    engine: &PersistenceEngine,
    request: CreateProjectRequest,
) -> Result<ProjectRecord, String> {
    engine.require_durable_store("create project")?;
    let project_id = ProjectId::new().to_string();
    let name = clean_text(&request.name, "name", 120, true)?;
    let description = clean_text(&request.description, "description", 2_000, false)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO projects (project_id, name, description, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?4)", params![project_id, name, description, now]).map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO project_policy (project_id, data_policy, updated_at_ms) VALUES (?1, ?2, ?3)",
        params![project_id, request.data_policy.as_str(), now],
    )
    .map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO project_instructions (project_id, instructions, updated_at_ms) VALUES (?1, '', ?2)", params![project_id, now]).map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    get(engine, &project_id)
}

pub(super) fn list(
    engine: &PersistenceEngine,
    include_archived: bool,
) -> Result<Vec<ProjectRecord>, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let sql = format!(
        "{PROJECT_SELECT} WHERE p.project_id <> ?1 {} ORDER BY p.archived_at_ms IS NOT NULL, p.updated_at_ms DESC",
        if include_archived {
            ""
        } else {
            "AND p.archived_at_ms IS NULL"
        }
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![INTERNAL_LOCAL_FILES_PROJECT_ID], project_from_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub(super) fn get(engine: &PersistenceEngine, raw_id: &str) -> Result<ProjectRecord, String> {
    let project_id = ProjectId::parse(raw_id)?.to_string();
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            &format!("{PROJECT_SELECT} WHERE p.project_id=?1"),
            params![project_id],
            project_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project was not found.".to_string())
}

pub(super) fn update(
    engine: &PersistenceEngine,
    request: UpdateProjectRequest,
) -> Result<ProjectRecord, String> {
    let id = user_managed_project_id(&request.project_id)?;
    let name = clean_text(&request.name, "name", 120, true)?;
    let description = clean_text(&request.description, "description", 2_000, false)?;
    let changed = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE projects SET name=?2, description=?3, updated_at_ms=?4 WHERE project_id=?1",
            params![
                id,
                name,
                description,
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("Project was not found.".to_string());
    }
    get(engine, &id)
}

pub(super) fn archive(engine: &PersistenceEngine, raw_id: &str) -> Result<ProjectRecord, String> {
    let id = user_managed_project_id(raw_id)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE projects SET archived_at_ms=?2, updated_at_ms=?2 WHERE project_id=?1",
            params![id, now],
        )
        .map_err(|error| error.to_string())?;
    get(engine, &id)
}

pub(super) fn deletion_preview(
    engine: &PersistenceEngine,
    raw_id: &str,
    app_data: &Path,
) -> Result<ProjectDeletionPreview, String> {
    super::deletion::deletion_preview(engine, raw_id, app_data)
}

pub(super) fn delete(
    engine: &PersistenceEngine,
    knowledge: &crate::knowledge::KnowledgeStore,
    memory: &crate::memory_ledger::MemoryLedger,
    app_data: &Path,
    request: DeleteProjectRequest,
) -> Result<ProjectDeletionPreview, String> {
    super::deletion::delete(engine, knowledge, memory, app_data, request)
}
fn source_from_row(row: &Row<'_>) -> rusqlite::Result<ProjectSourceRecord> {
    Ok(ProjectSourceRecord {
        source_id: row.get(0)?,
        project_id: row.get(1)?,
        source_kind: row.get(2)?,
        canonical_path: row.get(3)?,
        grant_state: row.get(4)?,
        indexing_state: row.get(5)?,
        file_count: row.get::<_, i64>(6)? as usize,
        last_indexed_at_ms: row.get(7)?,
        failure_code: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

pub(super) fn attach_source(
    engine: &PersistenceEngine,
    request: AttachProjectSourceRequest,
) -> Result<ProjectSourceRecord, String> {
    let project_id = user_managed_project_id(&request.project_id)?;
    get(engine, &project_id)?;
    if !matches!(
        request.source_kind.as_str(),
        "local_folder" | "knowledge_directory"
    ) {
        return Err("Unsupported project source kind.".to_string());
    }
    if request.grant_reference.len() != 64
        || !request
            .grant_reference
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Project source requires a native picker grant reference.".to_string());
    }
    let canonical = fs::canonicalize(request.path)
        .map_err(|_| "The approved folder is unavailable.".to_string())?;
    if !canonical.is_dir()
        || canonical
            .symlink_metadata()
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("Project source must be an approved physical folder.".to_string());
    }
    let source_id = ProjectId::new()
        .to_string()
        .replacen("project_", "source_", 1);
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection.execute("INSERT INTO project_sources (source_id, project_id, source_kind, canonical_path, grant_reference, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)", params![source_id, project_id, request.source_kind, canonical.to_string_lossy(), request.grant_reference, now]).map_err(|error| error.to_string())?;
    source_by_id(&connection, &project_id, &source_id)
}

pub(super) fn attach_picked_root(
    engine: &PersistenceEngine,
    raw_project_id: &str,
    picked_path: &Path,
) -> Result<ProjectSourceRecord, String> {
    let project_id = validate_user_project(engine, raw_project_id)?;
    let metadata = fs::symlink_metadata(picked_path)
        .map_err(|_| "The chosen Project folder is unavailable.".to_string())?;
    let canonical = fs::canonicalize(picked_path)
        .map_err(|_| "The chosen Project folder is unavailable.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !canonical.is_dir() {
        return Err("Choose a physical folder for this Project.".to_string());
    }
    let mut grant = [0_u8; 32];
    OsRng.fill_bytes(&mut grant);
    let grant_reference = hex::encode(grant);
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let existing_root: Option<String> = transaction
        .query_row(
            "SELECT source_id FROM project_sources WHERE project_id=?1 AND source_kind='local_folder' ORDER BY updated_at_ms DESC LIMIT 1",
            params![project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let source_id = existing_root.unwrap_or_else(|| {
        ProjectId::new()
            .to_string()
            .replacen("project_", "source_", 1)
    });
    let path = canonical.to_string_lossy().to_string();
    let conflicts: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM project_sources WHERE project_id=?1 AND canonical_path=?2 AND source_id!=?3)",
            params![project_id, path, source_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if conflicts {
        return Err("That folder is already attached as Knowledge. Remove it from Knowledge before using it as the Project folder.".to_string());
    }
    transaction
        .execute(
            "INSERT INTO project_sources (source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,indexing_state,file_count,last_indexed_at_ms,failure_code,created_at_ms,updated_at_ms)
             VALUES (?1,?2,'local_folder',?3,?4,'active','ready',0,NULL,NULL,?5,?5)
             ON CONFLICT(source_id) DO UPDATE SET canonical_path=excluded.canonical_path,grant_reference=excluded.grant_reference,grant_state='active',indexing_state='ready',file_count=0,last_indexed_at_ms=NULL,failure_code=NULL,updated_at_ms=excluded.updated_at_ms",
            params![source_id, project_id, path, grant_reference, now],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    source_by_id(&connection, &project_id, &source_id)
}

fn source_by_id(
    connection: &rusqlite::Connection,
    project_id: &str,
    source_id: &str,
) -> Result<ProjectSourceRecord, String> {
    connection.query_row("SELECT source_id, project_id, source_kind, canonical_path, grant_state, indexing_state, file_count, last_indexed_at_ms, failure_code, updated_at_ms FROM project_sources WHERE project_id=?1 AND source_id=?2", params![project_id, source_id], source_from_row).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Project source was not found.".to_string())
}

pub(super) fn list_sources(
    engine: &PersistenceEngine,
    raw_id: &str,
) -> Result<Vec<ProjectSourceRecord>, String> {
    let id = user_managed_project_id(raw_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare("SELECT source_id, project_id, source_kind, canonical_path, grant_state, indexing_state, file_count, last_indexed_at_ms, failure_code, updated_at_ms FROM project_sources WHERE project_id=?1 ORDER BY updated_at_ms DESC").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![id], source_from_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StartupKnowledgeRefresh {
    pub(crate) refreshed: usize,
    pub(crate) empty: usize,
    pub(crate) failed: usize,
}

pub(crate) fn refresh_active_knowledge_sources_at_startup(
    engine: &PersistenceEngine,
    knowledge: &crate::knowledge::KnowledgeStore,
    gemma: crate::gemma::GemmaService,
) -> Result<StartupKnowledgeRefresh, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let sources = {
        let mut statement = connection
            .prepare(
                "SELECT s.source_id,s.project_id,s.canonical_path
                 FROM project_sources s
                 JOIN projects p ON p.project_id=s.project_id
                 WHERE s.source_kind='knowledge_directory'
                   AND s.grant_state='active'
                   AND p.archived_at_ms IS NULL
                   AND s.project_id<>?1
                 ORDER BY s.created_at_ms",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![INTERNAL_LOCAL_FILES_PROJECT_ID], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    drop(connection);

    let mut summary = StartupKnowledgeRefresh::default();
    for (source_id, project_id, canonical_path) in sources {
        let now = crate::foundation::clock::unix_time_ms_i64();
        engine
            .open_connection()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE project_sources SET indexing_state='indexing',failure_code=NULL,updated_at_ms=?3 WHERE project_id=?1 AND source_id=?2",
                params![project_id, source_id, now],
            )
            .map_err(|error| error.to_string())?;

        match crate::knowledge::ingest_persisted_project_directory(
            knowledge,
            gemma.clone(),
            &project_id,
            Path::new(&canonical_path),
        ) {
            Ok(file_count) => {
                let refreshed_at = crate::foundation::clock::unix_time_ms_i64();
                engine
                    .open_connection()
                    .map_err(|error| error.to_string())?
                    .execute(
                        "UPDATE project_sources SET indexing_state='ready',file_count=?3,last_indexed_at_ms=?4,failure_code=NULL,updated_at_ms=?4 WHERE project_id=?1 AND source_id=?2",
                        params![project_id, source_id, file_count as i64, refreshed_at],
                    )
                    .map_err(|error| error.to_string())?;
                summary.refreshed += 1;
                if file_count == 0 {
                    summary.empty += 1;
                }
            }
            Err(error) => {
                let failed_at = crate::foundation::clock::unix_time_ms_i64();
                let unavailable = error.code == "knowledge_io_failed";
                engine
                    .open_connection()
                    .map_err(|db_error| db_error.to_string())?
                    .execute(
                        "UPDATE project_sources SET grant_state=CASE WHEN ?3 THEN 'unavailable' ELSE grant_state END,indexing_state='failed',failure_code=?4,updated_at_ms=?5 WHERE project_id=?1 AND source_id=?2",
                        params![project_id, source_id, unavailable, error.code, failed_at],
                    )
                    .map_err(|db_error| db_error.to_string())?;
                summary.failed += 1;
            }
        }
    }
    Ok(summary)
}

fn count_files(root: &Path, visited: &mut usize) -> Result<(), String> {
    if *visited >= 10_000 {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|_| {
        "Approved folder permission was revoked or the folder is unavailable.".to_string()
    })? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            count_files(&entry.path(), visited)?;
        } else if kind.is_file() && crate::knowledge::is_supported_knowledge_file(&entry.path()) {
            *visited += 1;
        }
        if *visited >= 10_000 {
            break;
        }
    }
    Ok(())
}

pub(super) fn refresh_source(
    engine: &PersistenceEngine,
    request: ProjectSourceRequest,
) -> Result<ProjectSourceRecord, String> {
    let id = user_managed_project_id(&request.project_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let source = source_by_id(&connection, &id, &request.source_id)?;
    if source.grant_state != "active" {
        return Err(
            "Folder permission is revoked. Choose the folder again to restore access.".to_string(),
        );
    }
    let path = PathBuf::from(&source.canonical_path);
    let mut count = 0;
    let result = fs::canonicalize(&path)
        .map_err(|_| "Approved folder permission was revoked or the folder moved.".to_string())
        .and_then(|current| {
            if current != path {
                return Err(
                    "Approved folder identity changed. Choose it again to continue.".to_string(),
                );
            }
            count_files(&current, &mut count)
        });
    let now = crate::foundation::clock::unix_time_ms_i64();
    match result {
        Ok(()) => {
            connection.execute("UPDATE project_sources SET indexing_state='ready', file_count=?3, last_indexed_at_ms=?4, failure_code=NULL, updated_at_ms=?4 WHERE project_id=?1 AND source_id=?2", params![id, request.source_id, count as i64, now]).map_err(|error| error.to_string())?;
        }
        Err(error) => {
            connection.execute("UPDATE project_sources SET grant_state='unavailable', indexing_state='failed', failure_code='source_unavailable', updated_at_ms=?3 WHERE project_id=?1 AND source_id=?2", params![id, request.source_id, now]).map_err(|db_error| db_error.to_string())?;
            return Err(error);
        }
    }
    source_by_id(&connection, &id, &request.source_id)
}

pub(super) fn refresh_source_content(
    engine: &PersistenceEngine,
    knowledge: &crate::knowledge::KnowledgeStore,
    gemma: crate::gemma::GemmaService,
    request: ProjectSourceRequest,
) -> Result<ProjectSourceRecord, String> {
    let source = refresh_source(engine, request.clone())?;
    let result = crate::knowledge::ingest_persisted_project_directory(
        knowledge,
        gemma,
        &source.project_id,
        Path::new(&source.canonical_path),
    );
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    match result {
        Ok(0) if source.file_count > 0 => {
            connection
                .execute(
                    "UPDATE project_sources SET indexing_state='failed',failure_code='source_index_empty',updated_at_ms=?3 WHERE project_id=?1 AND source_id=?2",
                    params![source.project_id, source.source_id, now],
                )
                .map_err(|error| error.to_string())?;
            return Err("OOMU could not read the files in this folder.".to_string());
        }
        Ok(file_count) => {
            connection
                .execute(
                    "UPDATE project_sources SET indexing_state='ready',file_count=?3,last_indexed_at_ms=?4,failure_code=NULL,updated_at_ms=?4 WHERE project_id=?1 AND source_id=?2",
                    params![source.project_id, source.source_id, file_count as i64, now],
                )
                .map_err(|error| error.to_string())?;
        }
        Err(error) => {
            let unavailable = error.code == "knowledge_io_failed";
            connection
                .execute(
                    "UPDATE project_sources SET grant_state=CASE WHEN ?3 THEN 'unavailable' ELSE grant_state END,indexing_state='failed',failure_code=?4,updated_at_ms=?5 WHERE project_id=?1 AND source_id=?2",
                    params![source.project_id, source.source_id, unavailable, error.code, now],
                )
                .map_err(|db_error| db_error.to_string())?;
            return Err(error.message);
        }
    }
    source_by_id(&connection, &source.project_id, &source.source_id)
}

pub(super) fn revoke_source(
    engine: &PersistenceEngine,
    request: ProjectSourceRequest,
) -> Result<ProjectSourceRecord, String> {
    let id = user_managed_project_id(&request.project_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection.execute("UPDATE project_sources SET grant_reference='', grant_state='revoked', indexing_state='revoked', updated_at_ms=?3 WHERE project_id=?1 AND source_id=?2", params![id, request.source_id, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    source_by_id(&connection, &id, &request.source_id)
}

pub(super) fn set_instructions(
    engine: &PersistenceEngine,
    request: SetProjectInstructionsRequest,
) -> Result<ProjectRecord, String> {
    let id = user_managed_project_id(&request.project_id)?;
    let instructions = clean_text(&request.instructions, "instructions", 12_000, false)?;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE project_instructions SET instructions=?2, updated_at_ms=?3 WHERE project_id=?1",
            params![
                id,
                instructions,
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    get(engine, &id)
}

pub(super) fn set_policy(
    engine: &PersistenceEngine,
    request: SetProjectPolicyRequest,
) -> Result<ProjectRecord, String> {
    let id = user_managed_project_id(&request.project_id)?;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE project_policy SET data_policy=?2, updated_at_ms=?3 WHERE project_id=?1",
            params![
                id,
                request.data_policy.as_str(),
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    get(engine, &id)
}

pub(crate) fn bind_record(
    engine: &PersistenceEngine,
    request: BindProjectRecordRequest,
) -> Result<(), String> {
    let project_id = request
        .project_id
        .map(|project_id| user_managed_project_id(&project_id))
        .transpose()?;
    if let Some(id) = project_id.as_deref() {
        get(engine, id)?;
    }
    let (table, key) = match request.record_kind.as_str() {
        "chat_session" => ("chat_sessions", "id"),
        "workflow" => ("workflow_blueprints", "workflow_id"),
        "workflow_schedule" => ("workflow_schedules", "id"),
        _ => return Err("Unsupported project record kind.".to_string()),
    };
    let changed = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            &format!("UPDATE {table} SET project_id=?1 WHERE {key}=?2"),
            params![project_id, request.record_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The record to bind was not found.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
