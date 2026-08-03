use crate::db::{get_mod_db_connection, DatabaseError};
use crate::foundation::{
    clock::{unix_time_ms_i64 as unix_time_ms, unix_time_ns_from, wall_time_from},
    digest::{sha256, sha256_hex},
};
use crate::gemma::GemmaService;
use crate::security::firewall::default_workspace_id;
#[cfg(test)]
use crate::security::firewall::workspace_id_for_root;
use rand_core::{OsRng, RngCore};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
mod project_purge;
const VAULT_DIR: &str = ".oomu/vault";
const KNOWLEDGE_DB: &str = "knowledge.db";
const OPS_DB_FILE: &str = "oomu_ops.db";
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FILE_BYTES: u64 = 512 * 1024;
const CHUNK_LINES: usize = 80;
const CHUNK_OVERLAP_PERCENT: usize = 15;
const MAX_CHUNK_TOKENS: usize = 320;
#[cfg(test)]
const MAX_CONTEXT_TOKENS: usize = 1_400;
const SEMANTIC_DUPLICATE_THRESHOLD: f32 = 0.92;
const DEFAULT_KNOWLEDGE_MOD_ID: &str = "default";
const DEFAULT_KNOWLEDGE_PICKER_LIMIT: usize = 60;
const MAX_KNOWLEDGE_PICKER_FILES: usize = 240;
const MAX_KNOWLEDGE_DISCOVERY_ENTRIES: usize = 4_096;
const MAX_KNOWLEDGE_DISCOVERY_DEPTH: usize = 32;
const MAX_KNOWLEDGE_AGGREGATE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_LIVE_KNOWLEDGE_GRANTS: usize = 8;
const MAX_LIVE_KNOWLEDGE_FILES: usize = 480;
const MAX_LIVE_KNOWLEDGE_BYTES: u64 = 80 * 1024 * 1024;
const KNOWLEDGE_GRANT_TTL_MS: i64 = 5 * 60 * 1_000;
const PRIVATE_KNOWLEDGE_VAULT_ID: &str = "private://knowledge";
pub(crate) const KNOWLEDGE_SUPPORTED_FORMATS: &[&str] = &[
    "csv", "css", "html", "js", "json", "jsx", "md", "rs", "rst", "sql", "toml", "ts", "tsx",
    "txt", "xml", "yaml", "yml",
];
#[derive(Clone)]
pub struct KnowledgeStore {
    db_path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeIngestRequest {
    pub grant_id: String,
    pub session_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub mod_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChooseKnowledgeIngestDirectoryRequest {
    pub session_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub max_files: Option<usize>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeRemoveRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChooseKnowledgeIngestDirectoryResponse {
    pub grant_id: String,
    pub directory_name: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub expires_at_ms: i64,
    pub canonical_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeState {
    pub db_path: String,
    pub document_count: usize,
    pub chunk_count: usize,
    pub last_ingested_ms: Option<i64>,
    pub exclusions: Vec<String>,
    pub recent_context: Vec<KnowledgeDocument>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeDocument {
    pub path: String,
    pub content_hash: String,
    pub chunk_count: usize,
    pub modified_ms: i64,
    pub ingested_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeIngestResponse {
    pub indexed_files: usize,
    pub indexed_chunks: usize,
    pub skipped_files: usize,
    pub elapsed_ms: u128,
    pub db_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeContextBlock {
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub score: f32,
    pub semantic_relevance_score: f32,
    pub lexical_relevance_score: f32,
    pub overlap_percent: usize,
    pub token_count: usize,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ModKnowledgeContext {
    pub mod_id: String,
    pub blocks: Vec<KnowledgeContextBlock>,
}

#[derive(Debug, Serialize)]
pub struct KnowledgeError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

#[derive(Debug)]
struct CandidateFile {
    path: PathBuf,
    relative_path: String,
    modified_ms: i64,
    priority: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KnowledgeFileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    length: u64,
    modified_ns: u128,
}

impl KnowledgeFileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(unix_time_ns_from)
            .unwrap_or_default();
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            length: metadata.len(),
            modified_ns,
        }
    }
}

struct KnowledgeGrantedFile {
    path: PathBuf,
    relative_path: String,
    handle: fs::File,
    identity: KnowledgeFileIdentity,
    content_sha256: [u8; 32],
    modified_ms: i64,
}

struct KnowledgeIngestGrant {
    root_path: PathBuf,
    root_handle: fs::File,
    root_identity: KnowledgeFileIdentity,
    directory_name: String,
    files: Vec<KnowledgeGrantedFile>,
    session_id: String,
    turn_id: String,
    expires_at_ms: i64,
}

#[derive(Default)]
struct KnowledgeIngestGrantState {
    grants: HashMap<String, KnowledgeIngestGrant>,
}

#[derive(Clone, Default)]
pub struct KnowledgeIngestGrantStore {
    state: Arc<Mutex<KnowledgeIngestGrantState>>,
}

#[derive(Debug)]
struct ConsumedKnowledgeFile {
    display_path: String,
    content: String,
    modified_ms: i64,
}

#[derive(Debug, Clone)]
struct KnowledgeScope {
    workspace_id: String,
    mod_id: String,
    workspace_root: String,
    project_id: Option<String>,
}

#[derive(Debug)]
struct StoredChunk {
    path: String,
    chunk_index: usize,
    line_start: usize,
    line_end: usize,
    snippet: String,
    embedding: Vec<f32>,
}

#[derive(Debug)]
struct ScoredChunk {
    block: KnowledgeContextBlock,
    embedding: Vec<f32>,
}

impl KnowledgeStore {
    pub fn initialize() -> Result<Self, String> {
        let vault_root = crate::launch_startup::vault_root(&crate::settings::app_data_root())
            .unwrap_or_else(|| home_dir().join(VAULT_DIR));
        let db_path = vault_root.join(KNOWLEDGE_DB);
        Self::initialize_at(db_path)
    }

    pub(crate) fn initialize_at(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let store = Self {
            db_path: Arc::new(db_path),
            write_lock: Arc::new(Mutex::new(())),
        };
        store.run_migrations().map_err(|error| error.to_string())?;
        Ok(store)
    }

    fn ingest_sync(
        &self,
        request: &KnowledgeIngestRequest,
        files: Vec<ConsumedKnowledgeFile>,
        gemma: GemmaService,
    ) -> Result<KnowledgeIngestResponse, KnowledgeError> {
        let started = Instant::now();
        let scope = match request.project_id.as_deref() {
            Some(project_id) => KnowledgeScope::from_project_id(project_id)?,
            None => KnowledgeScope::from_mod_id(request.mod_id.as_deref()),
        };

        let mut indexed_files = 0;
        let mut indexed_chunks = 0;
        let mut skipped_files = 0;
        for file in files {
            match self.index_content(
                &file.display_path,
                file.modified_ms,
                &file.content,
                &gemma,
                &scope,
            ) {
                Ok(chunks) => {
                    indexed_files += 1;
                    indexed_chunks += chunks;
                }
                Err(_) => skipped_files += 1,
            }
        }

        Ok(KnowledgeIngestResponse {
            indexed_files,
            indexed_chunks,
            skipped_files,
            elapsed_ms: started.elapsed().as_millis(),
            db_path: PRIVATE_KNOWLEDGE_VAULT_ID.to_string(),
        })
    }

    fn remove_sync(&self, path: String) -> Result<KnowledgeState, KnowledgeError> {
        let scope = KnowledgeScope::default();
        let relative = normalize_document_key(&path)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(KnowledgeError::database)?;
        connection
            .execute(
                "DELETE FROM knowledge_chunks WHERE path=?1 AND workspace_id=?2",
                params![relative, &scope.workspace_id],
            )
            .map_err(KnowledgeError::database)?;
        connection
            .execute(
                "DELETE FROM knowledge_documents WHERE path=?1 AND workspace_id=?2",
                params![relative, &scope.workspace_id],
            )
            .map_err(KnowledgeError::database)?;
        drop(connection);
        drop(_guard);
        self.state_sync()
    }

    fn state_sync(&self) -> Result<KnowledgeState, KnowledgeError> {
        let connection = self.open_connection().map_err(KnowledgeError::database)?;
        let scope = KnowledgeScope::default();
        let document_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM knowledge_documents WHERE workspace_id = ?1",
                params![&scope.workspace_id],
                |row| row.get(0),
            )
            .map_err(KnowledgeError::database)?;
        let chunk_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM knowledge_chunks WHERE workspace_id = ?1",
                params![&scope.workspace_id],
                |row| row.get(0),
            )
            .map_err(KnowledgeError::database)?;
        let last_ingested_ms = connection
            .query_row(
                "SELECT MAX(ingested_ms) FROM knowledge_documents WHERE workspace_id = ?1",
                params![&scope.workspace_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(KnowledgeError::database)?;
        Ok(KnowledgeState {
            db_path: PRIVATE_KNOWLEDGE_VAULT_ID.to_string(),
            document_count: document_count as usize,
            chunk_count: chunk_count as usize,
            last_ingested_ms,
            exclusions: default_exclusions(),
            recent_context: select_documents(&connection, &scope.workspace_id, 12)?,
        })
    }

    fn index_content(
        &self,
        relative_path: &str,
        modified_ms: i64,
        content: &str,
        gemma: &GemmaService,
        scope: &KnowledgeScope,
    ) -> Result<usize, KnowledgeError> {
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err(KnowledgeError::invalid(
                "File exceeds knowledge ingest limit.",
            ));
        }
        if content.trim().is_empty() {
            return Err(KnowledgeError::invalid("File is empty."));
        }
        let storage_path = scoped_storage_path(scope, relative_path);
        let content_hash = sha256_hex(content.as_bytes());
        let existing = self
            .open_connection()
            .map_err(KnowledgeError::database)?
            .query_row(
                "SELECT content_hash,chunk_count FROM knowledge_documents WHERE path=?1 AND workspace_id=?2",
                params![&storage_path, &scope.workspace_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(KnowledgeError::database)?;
        if existing
            .as_ref()
            .is_some_and(|(stored_hash, _)| stored_hash == &content_hash)
        {
            return Ok(existing
                .map(|(_, chunk_count)| chunk_count.max(0) as usize)
                .unwrap_or_default());
        }
        let chunks = sliding_chunks(&content)
            .into_iter()
            .map(|(line_start, line_end, snippet)| {
                let embedding = gemma
                    .embed_text_sync(&format!("{relative_path}\n{snippet}"))
                    .map_err(KnowledgeError::embedding)?;
                Ok((line_start, line_end, snippet, embedding.vector))
            })
            .collect::<Result<Vec<_>, KnowledgeError>>()?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection().map_err(KnowledgeError::database)?;
        let tx = connection.transaction().map_err(KnowledgeError::database)?;
        tx.execute(
            "
            INSERT INTO knowledge_documents (
                path, workspace_id, mod_id, workspace_root, project_id, content_hash, modified_ms, ingested_ms, chunk_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(path) DO UPDATE SET
                workspace_id=excluded.workspace_id,
                mod_id=excluded.mod_id,
                workspace_root=excluded.workspace_root,
                project_id=excluded.project_id,
                content_hash=excluded.content_hash,
                modified_ms=excluded.modified_ms,
                ingested_ms=excluded.ingested_ms,
                chunk_count=excluded.chunk_count
            ",
            params![
                &storage_path,
                &scope.workspace_id,
                &scope.mod_id,
                &scope.workspace_root,
                &scope.project_id,
                content_hash,
                modified_ms,
                unix_time_ms(),
                chunks.len() as i64
            ],
        )
        .map_err(KnowledgeError::database)?;
        tx.execute(
            "DELETE FROM knowledge_chunks WHERE path=?1 AND workspace_id=?2",
            params![&storage_path, &scope.workspace_id],
        )
        .map_err(KnowledgeError::database)?;
        for (index, (line_start, line_end, snippet, embedding)) in chunks.iter().enumerate() {
            tx.execute(
                "
                INSERT INTO knowledge_chunks (
                    path, workspace_id, mod_id, workspace_root, project_id, chunk_index, line_start, line_end, snippet,
                    embedding_json, embedding_source
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'local_llama_cpp')
                ",
                params![
                    &storage_path,
                    &scope.workspace_id,
                    &scope.mod_id,
                    &scope.workspace_root,
                    &scope.project_id,
                    index as i64,
                    *line_start as i64,
                    *line_end as i64,
                    snippet,
                    json_string(embedding)
                ],
            )
            .map_err(KnowledgeError::database)?;
        }
        tx.commit().map_err(KnowledgeError::database)?;
        Ok(chunks.len())
    }

    fn select_chunks(&self, scope: &KnowledgeScope) -> Result<Vec<StoredChunk>, KnowledgeError> {
        let connection = self.open_connection().map_err(KnowledgeError::database)?;
        let mut statement = connection
            .prepare(
                "
                SELECT path, chunk_index, line_start, line_end, snippet, embedding_json
                FROM knowledge_chunks
                WHERE workspace_id = ?1 AND mod_id = ?2 AND workspace_root = ?3 AND project_id IS ?4
                ORDER BY id DESC
                LIMIT 2000
                ",
            )
            .map_err(KnowledgeError::database)?;
        let rows = statement
            .query_map(
                params![
                    &scope.workspace_id,
                    &scope.mod_id,
                    &scope.workspace_root,
                    &scope.project_id
                ],
                |row| {
                    let embedding_json: String = row.get(5)?;
                    Ok(StoredChunk {
                        path: row.get(0)?,
                        chunk_index: row.get::<_, i64>(1)? as usize,
                        line_start: row.get::<_, i64>(2)? as usize,
                        line_end: row.get::<_, i64>(3)? as usize,
                        snippet: row.get(4)?,
                        embedding: serde_json::from_str(&embedding_json).unwrap_or_default(),
                    })
                },
            )
            .map_err(KnowledgeError::database)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(KnowledgeError::database)
    }

    fn run_migrations(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id = default_workspace_id();
        connection.execute_batch(
            &format!(
                "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS knowledge_documents (
                path TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                mod_id TEXT NOT NULL DEFAULT 'default',
                workspace_root TEXT NOT NULL DEFAULT '',
                project_id TEXT,
                content_hash TEXT NOT NULL,
                modified_ms INTEGER NOT NULL,
                ingested_ms INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS knowledge_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                mod_id TEXT NOT NULL DEFAULT 'default',
                workspace_root TEXT NOT NULL DEFAULT '',
                project_id TEXT,
                chunk_index INTEGER NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                snippet TEXT NOT NULL,
                embedding_json TEXT NOT NULL,
                embedding_source TEXT NOT NULL,
                UNIQUE(path, chunk_index),
                FOREIGN KEY(path) REFERENCES knowledge_documents(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_knowledge_chunks_path ON knowledge_chunks(path);
            CREATE INDEX IF NOT EXISTS idx_knowledge_documents_ingested ON knowledge_documents(ingested_ms);
            ",
                workspace_id, workspace_id
            ),
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_documents",
            "workspace_id",
            &format!(
                "ALTER TABLE knowledge_documents ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '{}'",
                workspace_id
            ),
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_documents",
            "mod_id",
            "ALTER TABLE knowledge_documents ADD COLUMN mod_id TEXT NOT NULL DEFAULT 'default'",
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_documents",
            "workspace_root",
            "ALTER TABLE knowledge_documents ADD COLUMN workspace_root TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_documents",
            "project_id",
            "ALTER TABLE knowledge_documents ADD COLUMN project_id TEXT",
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_chunks",
            "workspace_id",
            &format!(
                "ALTER TABLE knowledge_chunks ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '{}'",
                workspace_id
            ),
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_chunks",
            "mod_id",
            "ALTER TABLE knowledge_chunks ADD COLUMN mod_id TEXT NOT NULL DEFAULT 'default'",
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_chunks",
            "workspace_root",
            "ALTER TABLE knowledge_chunks ADD COLUMN workspace_root TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(
            &connection,
            "knowledge_chunks",
            "project_id",
            "ALTER TABLE knowledge_chunks ADD COLUMN project_id TEXT",
        )?;
        connection.execute_batch("CREATE INDEX IF NOT EXISTS idx_knowledge_documents_project ON knowledge_documents(project_id, ingested_ms DESC); CREATE INDEX IF NOT EXISTS idx_knowledge_chunks_project ON knowledge_chunks(project_id, id DESC);")?;
        let default_scope = KnowledgeScope::default();
        connection.execute(
            "UPDATE knowledge_documents SET workspace_id=?1 WHERE workspace_id = ''",
            params![&default_scope.workspace_id],
        )?;
        connection.execute(
            "UPDATE knowledge_documents SET mod_id=?1 WHERE mod_id = ''",
            params![&default_scope.mod_id],
        )?;
        connection.execute(
            "UPDATE knowledge_documents SET workspace_root=?1 WHERE workspace_root = ''",
            params![&default_scope.workspace_root],
        )?;
        connection.execute(
            "UPDATE knowledge_chunks SET workspace_id=?1 WHERE workspace_id = ''",
            params![&default_scope.workspace_id],
        )?;
        connection.execute(
            "UPDATE knowledge_chunks SET mod_id=?1 WHERE mod_id = ''",
            params![&default_scope.mod_id],
        )?;
        connection.execute(
            "UPDATE knowledge_chunks SET workspace_root=?1 WHERE workspace_root = ''",
            params![&default_scope.workspace_root],
        )?;
        connection.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_knowledge_chunks_scope
                ON knowledge_chunks(workspace_id, mod_id, workspace_root, id DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_documents_scope
                ON knowledge_documents(workspace_id, mod_id, workspace_root, ingested_ms DESC);
            ",
        )
    }

    fn open_connection(&self) -> rusqlite::Result<Connection> {
        let connection = Connection::open(self.db_path.as_ref())?;
        connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
        Ok(connection)
    }

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn score_chunks_for_query(
    query: &str,
    query_embedding: &[f32],
    chunks: Vec<StoredChunk>,
    limit: usize,
    max_context_tokens: usize,
) -> Vec<KnowledgeContextBlock> {
    let mut scored = chunks
        .into_iter()
        .filter(|chunk| chunk.embedding.len() == query_embedding.len())
        .map(|chunk| {
            let semantic = cosine_similarity(query_embedding, &chunk.embedding);
            let lexical = lexical_overlap(query, &chunk.snippet);
            let score = semantic + lexical + multi_document_bonus(chunk.chunk_index);
            let block = KnowledgeContextBlock {
                semantic_relevance_score: semantic,
                lexical_relevance_score: lexical,
                score,
                path: chunk.path,
                line_start: chunk.line_start,
                line_end: chunk.line_end,
                overlap_percent: CHUNK_OVERLAP_PERCENT,
                token_count: estimate_tokens(&chunk.snippet),
                snippet: chunk.snippet,
            };
            (block, chunk.embedding)
        })
        .map(|(block, embedding)| ScoredChunk { block, embedding })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .block
            .score
            .partial_cmp(&left.block.score)
            .unwrap_or(Ordering::Equal)
    });
    bounded_context_blocks(scored, limit, max_context_tokens)
}

pub(crate) fn sanitize_competitor_terms(snippet: &str) -> String {
    let mut cleansed = snippet.to_string();
    for (pattern, replacement) in competitor_term_patterns() {
        cleansed = pattern.replace_all(&cleansed, *replacement).into_owned();
    }
    cleansed
}

fn sanitize_context_blocks(blocks: &mut [KnowledgeContextBlock]) {
    for block in blocks {
        block.snippet = sanitize_competitor_terms(&block.snippet);
        block.token_count = estimate_tokens(&block.snippet);
    }
}

fn competitor_term_patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            vec![
                (
                    Regex::new(r"(?i)\bopen[- ]?claw\s+wrappers?\b")
                        .expect("competitor wrapper regex compiles"),
                    "OOMU high-performance kernel",
                ),
                (
                    Regex::new(r"(?i)\bopen[- ]?claw\s+configurations?\b")
                        .expect("competitor configuration regex compiles"),
                    "OOMU sovereign platform",
                ),
                (
                    Regex::new(r"(?i)\bopen[- ]?claw\b")
                        .expect("competitor platform regex compiles"),
                    "OOMU (custom local-first runtime)",
                ),
            ]
        })
        .as_slice()
}

impl KnowledgeScope {
    fn default() -> Self {
        let workspace_root = default_workspace_root();
        Self {
            workspace_id: default_workspace_id(),
            mod_id: DEFAULT_KNOWLEDGE_MOD_ID.to_string(),
            workspace_root,
            project_id: None,
        }
    }

    fn from_mod_id(mod_id: Option<&str>) -> Self {
        let mut scope = Self::default();
        scope.mod_id = normalized_scope_value(mod_id, &scope.mod_id);
        scope
    }

    fn from_project_id(project_id: &str) -> Result<Self, KnowledgeError> {
        let project_id =
            crate::p0_contracts::ProjectId::parse(project_id).map_err(KnowledgeError::invalid)?;
        let mut scope = Self::default();
        scope.mod_id = format!("project:{}", project_id.as_str());
        scope.project_id = Some(project_id.to_string());
        Ok(scope)
    }
}

fn normalized_scope_value(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .chars()
        .take(128)
        .collect()
}

fn default_workspace_root() -> String {
    project_root().to_string_lossy().replace('\\', "/")
}

fn scoped_storage_path(scope: &KnowledgeScope, relative_path: &str) -> String {
    scope.project_id.as_ref().map_or_else(
        || relative_path.to_string(),
        |project_id| format!("projects/{project_id}/{relative_path}"),
    )
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> rusqlite::Result<()> {
    if !column_exists(connection, table, column)? {
        connection.execute_batch(alter_sql)?;
    }
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[tauri::command]
pub async fn choose_knowledge_ingest_directory(
    request: ChooseKnowledgeIngestDirectoryRequest,
    grants: tauri::State<'_, KnowledgeIngestGrantStore>,
) -> Result<Option<ChooseKnowledgeIngestDirectoryResponse>, KnowledgeError> {
    validate_grant_scope(&request.session_id, &request.turn_id)?;
    let max_files = request.max_files.unwrap_or(DEFAULT_KNOWLEDGE_PICKER_LIMIT);
    if max_files == 0 {
        return Err(KnowledgeError::invalid(
            "Knowledge picker maxFiles must be greater than zero.",
        ));
    }
    if request
        .objective
        .as_deref()
        .is_some_and(|objective| objective.len() > 4_096)
    {
        return Err(KnowledgeError::invalid(
            "Knowledge picker objective exceeds the text limit.",
        ));
    }
    let Some(selected_directory) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let root = selected_directory.path().to_path_buf();
    let grant_store = grants.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        issue_knowledge_ingest_grant(
            &grant_store,
            &root,
            &request.session_id,
            &request.turn_id,
            request.objective.as_deref(),
            max_files.min(MAX_KNOWLEDGE_PICKER_FILES),
        )
        .map(Some)
    })
    .await
    .map_err(|error| KnowledgeError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn ingest_knowledge(
    request: KnowledgeIngestRequest,
    knowledge: tauri::State<'_, KnowledgeStore>,
    gemma: tauri::State<'_, GemmaService>,
    grants: tauri::State<'_, KnowledgeIngestGrantStore>,
) -> Result<KnowledgeIngestResponse, KnowledgeError> {
    let store = knowledge.inner().clone();
    let service = gemma.inner().clone();
    let grant_store = grants.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let files = consume_knowledge_ingest_grant(&grant_store, &request)?;
        store.ingest_sync(&request, files, service)
    })
    .await
    .map_err(|error| KnowledgeError::runtime(error.to_string()))?
}

pub(crate) fn ingest_persisted_project_directory(
    knowledge: &KnowledgeStore,
    gemma: GemmaService,
    project_id: &str,
    root: &Path,
) -> Result<usize, KnowledgeError> {
    let grants = KnowledgeIngestGrantStore::default();
    let scope = format!("startup-{}", random_grant_id());
    let grant = issue_knowledge_ingest_grant(
        &grants,
        root,
        &scope,
        &scope,
        None,
        MAX_KNOWLEDGE_PICKER_FILES,
    )?;
    let request = KnowledgeIngestRequest {
        grant_id: grant.grant_id,
        session_id: scope.clone(),
        turn_id: scope,
        mod_id: None,
        project_id: Some(project_id.to_string()),
    };
    let files = consume_knowledge_ingest_grant(&grants, &request)?;
    knowledge.ingest_sync(&request, files, gemma)?;
    Ok(grant.file_count)
}

#[tauri::command]
pub async fn remove_knowledge_document(
    request: KnowledgeRemoveRequest,
    knowledge: tauri::State<'_, KnowledgeStore>,
) -> Result<KnowledgeState, KnowledgeError> {
    let store = knowledge.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.remove_sync(request.path))
        .await
        .map_err(|error| KnowledgeError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_knowledge_state(
    knowledge: tauri::State<'_, KnowledgeStore>,
) -> Result<KnowledgeState, KnowledgeError> {
    knowledge.state_sync()
}

#[cfg(test)]
pub(crate) fn retrieve_blocks_for_gateway(
    knowledge: &KnowledgeStore,
    prompt: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<KnowledgeContextBlock>, KnowledgeError> {
    retrieve_blocks_for_gateway_with_embedding(
        knowledge,
        prompt,
        query_embedding,
        limit,
        MAX_CONTEXT_TOKENS,
        Instant::now(),
    )
}

pub(crate) fn retrieve_blocks_for_gateway_with_token_budget(
    knowledge: &KnowledgeStore,
    gemma: GemmaService,
    prompt: &str,
    limit: usize,
    max_context_tokens: usize,
) -> Result<Vec<KnowledgeContextBlock>, KnowledgeError> {
    let started = Instant::now();
    let query = prompt.trim();
    if query.is_empty() {
        return Err(KnowledgeError::invalid("Knowledge query cannot be empty."));
    }
    let query_embedding = gemma
        .embed_text_sync(query)
        .map_err(KnowledgeError::embedding)?;
    retrieve_blocks_for_gateway_with_embedding(
        knowledge,
        query,
        &query_embedding.vector,
        limit,
        max_context_tokens,
        started,
    )
}

pub(crate) fn retrieve_project_blocks_for_gateway_with_token_budget(
    knowledge: &KnowledgeStore,
    gemma: GemmaService,
    project_id: &str,
    prompt: &str,
    limit: usize,
    max_context_tokens: usize,
) -> Result<Vec<KnowledgeContextBlock>, KnowledgeError> {
    let started = Instant::now();
    let query = prompt.trim();
    if query.is_empty() {
        return Err(KnowledgeError::invalid("Knowledge query cannot be empty."));
    }
    let embedding = gemma
        .embed_text_sync(query)
        .map_err(KnowledgeError::embedding)?;
    let project_scope = KnowledgeScope::from_project_id(project_id)?;
    let mut project_blocks = score_chunks_for_query(
        query,
        &embedding.vector,
        knowledge.select_chunks(&project_scope)?,
        limit.clamp(1, 1_000),
        max_context_tokens.clamp(1, 500_000),
    );
    if project_blocks.len() < limit {
        let remaining = limit - project_blocks.len();
        let used_tokens = project_blocks
            .iter()
            .map(|block| block.token_count)
            .sum::<usize>();
        let global_blocks = score_chunks_for_query(
            query,
            &embedding.vector,
            knowledge.select_chunks(&KnowledgeScope::default())?,
            remaining,
            max_context_tokens.saturating_sub(used_tokens).max(1),
        );
        project_blocks.extend(global_blocks);
    }
    let mut blocks = project_blocks;
    sanitize_context_blocks(&mut blocks);
    log_rag_retrieval_audit(
        query,
        &blocks,
        started.elapsed().as_millis(),
        max_context_tokens,
    );
    Ok(blocks)
}

fn retrieve_blocks_for_gateway_with_embedding(
    knowledge: &KnowledgeStore,
    query: &str,
    query_embedding: &[f32],
    limit: usize,
    max_context_tokens: usize,
    started: Instant,
) -> Result<Vec<KnowledgeContextBlock>, KnowledgeError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(KnowledgeError::invalid("Knowledge query cannot be empty."));
    }
    if query_embedding.is_empty() {
        return Err(KnowledgeError::embedding(crate::gemma::GemmaError {
            code: "gemma_embedding_output_empty",
            message: "The local model returned an empty embedding tensor.".to_string(),
        }));
    }
    let scope = KnowledgeScope::default();
    let chunks = knowledge.select_chunks(&scope)?;
    let max_context_tokens = max_context_tokens.clamp(1, 500_000);
    let mut blocks = score_chunks_for_query(
        query,
        query_embedding,
        chunks,
        limit.clamp(1, 1_000),
        max_context_tokens,
    );
    sanitize_context_blocks(&mut blocks);
    log_rag_retrieval_audit(
        query,
        &blocks,
        started.elapsed().as_millis(),
        max_context_tokens,
    );
    Ok(blocks)
}

#[cfg(test)]
pub(crate) fn retrieve_mod_blocks_for_gateway(
    prompt: &str,
    query_embedding: &[f32],
    mod_ids: &[String],
    limit_per_mod: usize,
) -> Result<Vec<ModKnowledgeContext>, KnowledgeError> {
    retrieve_mod_blocks_for_gateway_with_embedding(
        prompt,
        query_embedding,
        mod_ids,
        limit_per_mod,
        MAX_CONTEXT_TOKENS,
    )
}

pub(crate) fn retrieve_mod_blocks_for_gateway_with_token_budget(
    gemma: GemmaService,
    prompt: &str,
    mod_ids: &[String],
    limit_per_mod: usize,
    max_context_tokens_per_mod: usize,
) -> Result<Vec<ModKnowledgeContext>, KnowledgeError> {
    let query = prompt.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let query_embedding = gemma
        .embed_text_sync(query)
        .map_err(KnowledgeError::embedding)?;
    retrieve_mod_blocks_for_gateway_with_embedding(
        query,
        &query_embedding.vector,
        mod_ids,
        limit_per_mod,
        max_context_tokens_per_mod,
    )
}

fn retrieve_mod_blocks_for_gateway_with_embedding(
    query: &str,
    query_embedding: &[f32],
    mod_ids: &[String],
    limit_per_mod: usize,
    max_context_tokens_per_mod: usize,
) -> Result<Vec<ModKnowledgeContext>, KnowledgeError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if query_embedding.is_empty() {
        return Err(KnowledgeError::embedding(crate::gemma::GemmaError {
            code: "gemma_embedding_output_empty",
            message: "The local model returned an empty embedding tensor.".to_string(),
        }));
    }
    let max_context_tokens_per_mod = max_context_tokens_per_mod.clamp(1, 250_000);
    let mut seen = HashSet::new();
    let mut contexts = Vec::new();
    for mod_id in mod_ids {
        let mod_id = mod_id.trim();
        if mod_id.is_empty() || !seen.insert(mod_id.to_string()) {
            continue;
        }
        match retrieve_isolated_mod_blocks(
            mod_id,
            query,
            query_embedding,
            limit_per_mod,
            max_context_tokens_per_mod,
        ) {
            Ok(blocks) if !blocks.is_empty() => contexts.push(ModKnowledgeContext {
                mod_id: mod_id.to_string(),
                blocks,
            }),
            Ok(_) => {}
            Err(error) => {
                eprintln!(
                    "OOMU_MOD_RAG_RETRIEVAL_SKIPPED mod_id={} code={} message={}",
                    mod_id, error.code, error.message
                );
            }
        }
    }
    Ok(contexts)
}

fn retrieve_isolated_mod_blocks(
    mod_id: &str,
    query: &str,
    query_embedding: &[f32],
    limit: usize,
    max_context_tokens: usize,
) -> Result<Vec<KnowledgeContextBlock>, KnowledgeError> {
    let connection = get_mod_db_connection(mod_id).map_err(KnowledgeError::database_boundary)?;
    let chunks = select_mod_chunks(&connection)?;
    let mut blocks = score_chunks_for_query(
        query,
        query_embedding,
        chunks,
        limit.clamp(1, 1_000),
        max_context_tokens,
    );
    sanitize_context_blocks(&mut blocks);
    Ok(blocks)
}

fn select_mod_chunks(connection: &Connection) -> Result<Vec<StoredChunk>, KnowledgeError> {
    let mut statement = connection
        .prepare(
            "
            SELECT path, chunk_index, line_start, line_end, snippet, embedding_json
            FROM knowledge_chunks
            ORDER BY id DESC
            LIMIT 2000
            ",
        )
        .map_err(KnowledgeError::database)?;
    let rows = statement
        .query_map([], |row| {
            let embedding_json: String = row.get(5)?;
            Ok(StoredChunk {
                path: row.get(0)?,
                chunk_index: row.get::<_, i64>(1)? as usize,
                line_start: row.get::<_, i64>(2)? as usize,
                line_end: row.get::<_, i64>(3)? as usize,
                snippet: row.get(4)?,
                embedding: serde_json::from_str(&embedding_json).unwrap_or_default(),
            })
        })
        .map_err(KnowledgeError::database)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(KnowledgeError::database)
}

#[cfg(test)]
pub(crate) fn source_tagged_prompt(prompt: &str, blocks: &[KnowledgeContextBlock]) -> String {
    if blocks.is_empty() {
        return prompt.to_string();
    }
    let context = source_blocks_text(blocks, MAX_CONTEXT_TOKENS);
    format!(
        "Use the following local OOMU context when relevant. Cite file paths and line numbers from each [SOURCE] block you use.\n\n{context}\n\nUser request:\n{prompt}"
    )
}

#[cfg(test)]
pub(crate) fn source_tagged_context(blocks: &[KnowledgeContextBlock]) -> Option<String> {
    source_tagged_context_with_token_budget(blocks, MAX_CONTEXT_TOKENS)
}

pub(crate) fn source_tagged_context_with_token_budget(
    blocks: &[KnowledgeContextBlock],
    max_context_tokens: usize,
) -> Option<String> {
    if blocks.is_empty() {
        return None;
    }
    let max_context_tokens = max_context_tokens.clamp(1, 500_000);
    Some(format!(
        "Use the following local OOMU knowledge vault context when relevant. Cite file paths and line numbers from each [SOURCE] block you use.\n\n{}",
        source_blocks_text(blocks, max_context_tokens)
    ))
}

#[cfg(test)]
pub(crate) fn mod_source_tagged_context(contexts: &[ModKnowledgeContext]) -> Option<String> {
    mod_source_tagged_context_with_token_budget(contexts, MAX_CONTEXT_TOKENS)
}

pub(crate) fn mod_source_tagged_context_with_token_budget(
    contexts: &[ModKnowledgeContext],
    max_context_tokens_per_mod: usize,
) -> Option<String> {
    if contexts.is_empty() {
        return None;
    }
    let max_context_tokens_per_mod = max_context_tokens_per_mod.clamp(1, 250_000);
    let context = contexts
        .iter()
        .filter(|context| !context.blocks.is_empty())
        .map(|context| {
            format!(
                "[MOD KNOWLEDGE BASE RETRIEVAL - SOURCE: {}]\n{}\n[END MOD KNOWLEDGE BASE RETRIEVAL]",
                context.mod_id,
                source_blocks_text(&context.blocks, max_context_tokens_per_mod)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!context.trim().is_empty()).then_some(context)
}

fn source_blocks_text(blocks: &[KnowledgeContextBlock], max_tokens: usize) -> String {
    let mut used_tokens = 0;
    blocks
        .iter()
        .filter_map(|block| {
            let remaining = max_tokens.saturating_sub(used_tokens);
            if remaining == 0 {
                return None;
            }
            let snippet = trim_to_token_bound(block.snippet.trim(), remaining);
            let token_count = estimate_tokens(&snippet);
            used_tokens += token_count;
            Some((block, snippet, token_count))
        })
        .map(|block| {
            let (block, snippet, token_count) = block;
            format!(
                "[SOURCE] {}:{}-{} score={:.3} semantic={:.3} lexical={:.3} overlap={} token_bound={}\n{}",
                block.path,
                block.line_start,
                block.line_end,
                block.score,
                block.semantic_relevance_score,
                block.lexical_relevance_score,
                block.overlap_percent,
                token_count,
                snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn issue_knowledge_ingest_grant(
    store: &KnowledgeIngestGrantStore,
    root: &Path,
    session_id: &str,
    turn_id: &str,
    objective: Option<&str>,
    max_files: usize,
) -> Result<ChooseKnowledgeIngestDirectoryResponse, KnowledgeError> {
    validate_grant_scope(session_id, turn_id)?;
    let root_metadata = fs::symlink_metadata(root).map_err(KnowledgeError::io)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(KnowledgeError::grant(
            "Knowledge selection must be a non-symlink directory.",
        ));
    }
    let root_path = fs::canonicalize(root).map_err(KnowledgeError::io)?;
    let root_handle = fs::File::open(&root_path).map_err(KnowledgeError::io)?;
    let root_identity =
        KnowledgeFileIdentity::from_metadata(&root_handle.metadata().map_err(KnowledgeError::io)?);
    revalidate_knowledge_path(&root_path, &root_handle, &root_identity, true)?;

    let candidates = discover_grant_candidates(&root_path, objective, max_files)?;
    let mut files = Vec::with_capacity(candidates.len());
    let mut total_bytes = 0_u64;
    for candidate in candidates {
        revalidate_knowledge_path(&root_path, &root_handle, &root_identity, true)?;
        let metadata = fs::symlink_metadata(&candidate.path).map_err(KnowledgeError::io)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(KnowledgeError::grant(
                "A selected knowledge file changed during grant issuance.",
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let canonical_path = fs::canonicalize(&candidate.path).map_err(KnowledgeError::io)?;
        if !canonical_path.starts_with(&root_path) {
            return Err(KnowledgeError::grant(
                "A selected knowledge file escaped the chosen directory.",
            ));
        }
        let mut handle = fs::File::open(&canonical_path).map_err(KnowledgeError::io)?;
        let handle_metadata = handle.metadata().map_err(KnowledgeError::io)?;
        let identity = KnowledgeFileIdentity::from_metadata(&handle_metadata);
        revalidate_knowledge_path(&canonical_path, &handle, &identity, false)?;
        let bytes = read_bounded_granted_file(&mut handle)?;
        if std::str::from_utf8(&bytes).is_err() {
            continue;
        }
        if total_bytes.saturating_add(bytes.len() as u64) > MAX_KNOWLEDGE_AGGREGATE_BYTES {
            break;
        }
        let content_sha256 = *sha256(&bytes).as_bytes();
        total_bytes += bytes.len() as u64;
        files.push(KnowledgeGrantedFile {
            path: canonical_path,
            relative_path: candidate.relative_path,
            handle,
            identity,
            content_sha256,
            modified_ms: handle_metadata
                .modified()
                .map(|value| wall_time_from(value).get())
                .unwrap_or(candidate.modified_ms),
        });
    }
    revalidate_knowledge_path(&root_path, &root_handle, &root_identity, true)?;
    let directory_name = sanitized_directory_name(&root_path);
    let expires_at_ms = unix_time_ms().saturating_add(KNOWLEDGE_GRANT_TTL_MS);
    let grant_id = random_grant_id();
    let response = ChooseKnowledgeIngestDirectoryResponse {
        grant_id: grant_id.clone(),
        directory_name: directory_name.clone(),
        file_count: files.len(),
        total_bytes,
        expires_at_ms,
        canonical_path: root_path.to_string_lossy().to_string(),
    };
    let mut state = store
        .state
        .lock()
        .map_err(|_| KnowledgeError::grant("Knowledge grant store is unavailable."))?;
    let now = unix_time_ms();
    state.grants.retain(|_, grant| grant.expires_at_ms > now);
    let live_file_count = state
        .grants
        .values()
        .map(|grant| grant.files.len())
        .sum::<usize>();
    let live_byte_count = state
        .grants
        .values()
        .flat_map(|grant| grant.files.iter())
        .map(|file| file.identity.length)
        .sum::<u64>();
    if state.grants.len() >= MAX_LIVE_KNOWLEDGE_GRANTS
        || live_file_count.saturating_add(files.len()) > MAX_LIVE_KNOWLEDGE_FILES
        || live_byte_count.saturating_add(total_bytes) > MAX_LIVE_KNOWLEDGE_BYTES
    {
        return Err(KnowledgeError::grant(
            "Knowledge grant capacity is exhausted; consume or let existing grants expire.",
        ));
    }
    state.grants.insert(
        grant_id,
        KnowledgeIngestGrant {
            root_path,
            root_handle,
            root_identity,
            directory_name,
            files,
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            expires_at_ms,
        },
    );
    Ok(response)
}

fn consume_knowledge_ingest_grant(
    store: &KnowledgeIngestGrantStore,
    request: &KnowledgeIngestRequest,
) -> Result<Vec<ConsumedKnowledgeFile>, KnowledgeError> {
    validate_grant_scope(&request.session_id, &request.turn_id)?;
    if request.grant_id.len() != 64
        || !request
            .grant_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(KnowledgeError::grant("Knowledge grant is invalid."));
    }
    let mut grant = store
        .state
        .lock()
        .map_err(|_| KnowledgeError::grant("Knowledge grant store is unavailable."))?
        .grants
        .remove(&request.grant_id)
        .ok_or_else(|| KnowledgeError::grant("Knowledge grant is invalid or already consumed."))?;
    if grant.expires_at_ms <= unix_time_ms() {
        return Err(KnowledgeError::grant("Knowledge grant has expired."));
    }
    if grant.session_id != request.session_id || grant.turn_id != request.turn_id {
        return Err(KnowledgeError::grant(
            "Knowledge grant does not match this session and turn.",
        ));
    }

    let mut consumed = Vec::with_capacity(grant.files.len());
    let mut aggregate_bytes = 0_u64;
    for file in &mut grant.files {
        revalidate_knowledge_path(
            &grant.root_path,
            &grant.root_handle,
            &grant.root_identity,
            true,
        )?;
        if !file.path.starts_with(&grant.root_path) {
            return Err(KnowledgeError::grant(
                "Knowledge grant file escaped the selected directory.",
            ));
        }
        revalidate_knowledge_path(&file.path, &file.handle, &file.identity, false)?;
        let bytes = read_bounded_granted_file(&mut file.handle)?;
        if sha256(&bytes).as_bytes() != &file.content_sha256 {
            return Err(KnowledgeError::grant(
                "Knowledge grant file contents changed after selection.",
            ));
        }
        revalidate_knowledge_path(&file.path, &file.handle, &file.identity, false)?;
        aggregate_bytes = aggregate_bytes.saturating_add(bytes.len() as u64);
        if aggregate_bytes > MAX_KNOWLEDGE_AGGREGATE_BYTES {
            return Err(KnowledgeError::grant(
                "Knowledge grant exceeded the aggregate byte limit.",
            ));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| KnowledgeError::grant("Knowledge grant contained a non-text file."))?;
        consumed.push(ConsumedKnowledgeFile {
            display_path: format!("{}/{}", grant.directory_name, file.relative_path),
            content,
            modified_ms: file.modified_ms,
        });
    }
    revalidate_knowledge_path(
        &grant.root_path,
        &grant.root_handle,
        &grant.root_identity,
        true,
    )?;
    Ok(consumed)
}

fn discover_grant_candidates(
    root: &Path,
    objective: Option<&str>,
    max_files: usize,
) -> Result<Vec<CandidateFile>, KnowledgeError> {
    let ignore = IgnoreRules {
        patterns: default_exclusions(),
    };
    let objective_terms = objective_terms(objective.unwrap_or_default());
    let mut candidates = Vec::new();
    let mut visited_entries = 0_usize;
    visit_grant_directory(
        root,
        root,
        0,
        &ignore,
        &objective_terms,
        &mut visited_entries,
        &mut candidates,
    )?;
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| right.modified_ms.cmp(&left.modified_ms))
    });
    candidates.truncate(max_files);
    Ok(candidates)
}

fn visit_grant_directory(
    root: &Path,
    dir: &Path,
    depth: usize,
    ignore: &IgnoreRules,
    objective_terms: &HashSet<String>,
    visited_entries: &mut usize,
    candidates: &mut Vec<CandidateFile>,
) -> Result<(), KnowledgeError> {
    if depth > MAX_KNOWLEDGE_DISCOVERY_DEPTH {
        return Err(KnowledgeError::grant(
            "Knowledge directory exceeded the discovery depth limit.",
        ));
    }
    let entries = fs::read_dir(dir).map_err(KnowledgeError::io)?;
    for entry in entries {
        let entry = entry.map_err(KnowledgeError::io)?;
        *visited_entries = visited_entries.saturating_add(1);
        if *visited_entries > MAX_KNOWLEDGE_DISCOVERY_ENTRIES {
            return Err(KnowledgeError::grant(
                "Knowledge directory exceeded the discovery entry limit.",
            ));
        }
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if ignore.is_ignored(relative) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(KnowledgeError::io)?;
        if metadata.file_type().is_symlink() {
            return Err(KnowledgeError::grant(
                "Knowledge directories may not contain followed symlinks.",
            ));
        }
        let canonical_path = fs::canonicalize(&path).map_err(KnowledgeError::io)?;
        if !canonical_path.starts_with(root) {
            return Err(KnowledgeError::grant(
                "Knowledge discovery escaped the selected directory.",
            ));
        }
        if metadata.is_dir() {
            visit_grant_directory(
                root,
                &canonical_path,
                depth + 1,
                ignore,
                objective_terms,
                visited_entries,
                candidates,
            )?;
            continue;
        }
        if !metadata.is_file() || !is_supported_knowledge_file(&canonical_path) {
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let modified_ms = metadata
            .modified()
            .map(|value| wall_time_from(value).get())
            .map_err(KnowledgeError::io)?;
        let mut priority = modified_ms;
        let relative_text = relative.to_string_lossy().to_lowercase();
        for term in objective_terms {
            if relative_text.contains(term) {
                priority += 86_400_000;
            }
        }
        candidates.push(CandidateFile {
            path: canonical_path,
            relative_path: relative.to_string_lossy().replace('\\', "/"),
            modified_ms,
            priority,
        });
    }
    Ok(())
}

fn revalidate_knowledge_path(
    path: &Path,
    handle: &fs::File,
    expected: &KnowledgeFileIdentity,
    expect_directory: bool,
) -> Result<(), KnowledgeError> {
    let path_metadata = fs::symlink_metadata(path).map_err(KnowledgeError::io)?;
    let expected_type = if expect_directory {
        path_metadata.is_dir()
    } else {
        path_metadata.is_file()
    };
    if path_metadata.file_type().is_symlink() || !expected_type {
        return Err(KnowledgeError::grant(
            "Knowledge grant target type changed after selection.",
        ));
    }
    if fs::canonicalize(path).map_err(KnowledgeError::io)? != path {
        return Err(KnowledgeError::grant(
            "Knowledge grant target path changed after selection.",
        ));
    }
    let handle_metadata = handle.metadata().map_err(KnowledgeError::io)?;
    if KnowledgeFileIdentity::from_metadata(&path_metadata) != *expected
        || KnowledgeFileIdentity::from_metadata(&handle_metadata) != *expected
    {
        return Err(KnowledgeError::grant(
            "Knowledge grant target identity changed after selection.",
        ));
    }
    Ok(())
}

fn read_bounded_granted_file(handle: &mut fs::File) -> Result<Vec<u8>, KnowledgeError> {
    handle
        .seek(SeekFrom::Start(0))
        .map_err(KnowledgeError::io)?;
    let mut bytes = Vec::new();
    handle
        .take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(KnowledgeError::io)?;
    handle
        .seek(SeekFrom::Start(0))
        .map_err(KnowledgeError::io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(KnowledgeError::grant(
            "Knowledge file exceeded the per-file byte limit.",
        ));
    }
    Ok(bytes)
}

fn validate_grant_scope(session_id: &str, turn_id: &str) -> Result<(), KnowledgeError> {
    if session_id.trim().is_empty()
        || turn_id.trim().is_empty()
        || session_id.len() > 256
        || turn_id.len() > 256
    {
        return Err(KnowledgeError::grant(
            "Knowledge grants require bounded session and turn identifiers.",
        ));
    }
    Ok(())
}

fn random_grant_id() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn sanitized_directory_name(root: &Path) -> String {
    let value = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("knowledge");
    let sanitized = value
        .chars()
        .take(80)
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.trim().is_empty() {
        "knowledge".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn is_supported_knowledge_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    KNOWLEDGE_SUPPORTED_FORMATS.contains(&extension.to_ascii_lowercase().as_str())
}

fn sliding_chunks(content: &str) -> Vec<(usize, usize, String)> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let overlap_lines = ((CHUNK_LINES * CHUNK_OVERLAP_PERCENT) / 100).max(1);
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let snippet = trim_to_token_bound(&lines[start..end].join("\n"), MAX_CHUNK_TOKENS);
        chunks.push((start + 1, end, snippet));
        if end == lines.len() {
            break;
        }
        start += CHUNK_LINES.saturating_sub(overlap_lines).max(1);
    }
    chunks
}

struct IgnoreRules {
    patterns: Vec<String>,
}

impl IgnoreRules {
    fn is_ignored(&self, relative: &Path) -> bool {
        let relative = relative.to_string_lossy().replace('\\', "/");
        self.patterns.iter().any(|pattern| {
            let normalized = pattern.trim_end_matches('/').trim_start_matches('/');
            relative == normalized
                || relative.starts_with(&format!("{normalized}/"))
                || relative.contains(&format!("/{normalized}/"))
                || (normalized.starts_with("*.") && relative.ends_with(&normalized[1..]))
        })
    }
}

fn default_exclusions() -> Vec<String> {
    vec![
        ".git".to_string(),
        ".next".to_string(),
        "node_modules".to_string(),
        "src-tauri/target".to_string(),
        "models".to_string(),
        "app_data".to_string(),
        "dist".to_string(),
        "build".to_string(),
        ".env".to_string(),
        ".env.local".to_string(),
    ]
}

fn select_documents(
    connection: &Connection,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<KnowledgeDocument>, KnowledgeError> {
    let mut statement = connection
        .prepare(
            "
            SELECT path, content_hash, chunk_count, modified_ms, ingested_ms
            FROM knowledge_documents
            WHERE workspace_id = ?1
            ORDER BY ingested_ms DESC
            LIMIT ?2
            ",
        )
        .map_err(KnowledgeError::database)?;
    let rows = statement
        .query_map(params![workspace_id, limit as i64], |row| {
            Ok(KnowledgeDocument {
                path: row.get(0)?,
                content_hash: row.get(1)?,
                chunk_count: row.get::<_, i64>(2)? as usize,
                modified_ms: row.get(3)?,
                ingested_ms: row.get(4)?,
            })
        })
        .map_err(KnowledgeError::database)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(KnowledgeError::database)
}

fn cosine_similarity(query: &[f32], candidate: &[f32]) -> f32 {
    query
        .iter()
        .zip(candidate.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn vector_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn lexical_overlap(query: &str, snippet: &str) -> f32 {
    let query_terms = objective_terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let snippet = snippet.to_lowercase();
    let hits = query_terms
        .iter()
        .filter(|term| snippet.contains(term.as_str()))
        .count();
    hits as f32 / query_terms.len() as f32
}

fn bounded_context_blocks(
    scored: Vec<ScoredChunk>,
    limit: usize,
    max_context_tokens: usize,
) -> Vec<KnowledgeContextBlock> {
    let mut selected: Vec<ScoredChunk> = Vec::new();
    let mut deferred: Vec<ScoredChunk> = Vec::new();
    let mut selected_paths = HashSet::new();
    let mut token_budget = 0;
    let max_context_tokens = max_context_tokens.max(1);

    for candidate in scored {
        let token_count = candidate.block.token_count;
        if token_budget + token_count > max_context_tokens {
            deferred.push(candidate);
            continue;
        }
        let duplicate = selected.iter().any(|existing| {
            existing.block.path == candidate.block.path
                && vector_similarity(&existing.embedding, &candidate.embedding)
                    >= SEMANTIC_DUPLICATE_THRESHOLD
        });
        if duplicate {
            deferred.push(candidate);
            continue;
        }

        if selected.len() < limit {
            token_budget += token_count;
            selected_paths.insert(candidate.block.path.clone());
            selected.push(candidate);
        } else {
            deferred.push(candidate);
        }
    }

    if selected_paths.len() < 2 {
        for candidate in deferred {
            if selected.len() >= limit {
                break;
            }
            if selected
                .iter()
                .any(|item| item.block.path == candidate.block.path)
            {
                continue;
            }
            if token_budget + candidate.block.token_count > max_context_tokens {
                continue;
            }
            token_budget += candidate.block.token_count;
            selected.push(candidate);
        }
    }

    selected.into_iter().map(|item| item.block).collect()
}

fn multi_document_bonus(chunk_index: usize) -> f32 {
    if chunk_index == 0 {
        0.03
    } else {
        0.0
    }
}

fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}

fn trim_to_token_bound(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

fn objective_terms(input: &str) -> HashSet<String> {
    input
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .map(str::to_lowercase)
        .filter(|term| term.len() > 2)
        .filter(|term| {
            !matches!(
                term.as_str(),
                "the" | "and" | "for" | "with" | "from" | "into" | "this" | "that"
            )
        })
        .collect()
}

fn normalize_document_key(path: &str) -> Result<String, KnowledgeError> {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(KnowledgeError::invalid(
            "Knowledge document key must be a non-empty relative identifier.",
        ));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    KnowledgeError::invalid("Knowledge document key must be valid UTF-8.")
                })?;
                if value.is_empty() {
                    return Err(KnowledgeError::invalid(
                        "Knowledge document key contains an empty component.",
                    ));
                }
                components.push(value);
            }
            _ => {
                return Err(KnowledgeError::invalid(
                    "Knowledge document key cannot contain traversal components.",
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(KnowledgeError::invalid(
            "Knowledge document key must be non-empty.",
        ));
    }
    Ok(components.join("/"))
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[]".to_string())
}

fn log_rag_retrieval_audit(
    query: &str,
    blocks: &[KnowledgeContextBlock],
    elapsed_ms: u128,
    max_context_tokens: usize,
) {
    let db_path = project_root().join(OPS_DB_FILE);
    if let Some(parent) = db_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(connection) = crate::db::open_ops_database_connection(&db_path) else {
        return;
    };
    if migrate_rag_audit(&connection).is_err() {
        return;
    }
    let trace_json = serde_json::json!({
        "query_hash": sha256_hex(query.as_bytes()),
        "elapsed_ms": elapsed_ms,
        "max_context_tokens": max_context_tokens,
        "chunk_overlap_percent": CHUNK_OVERLAP_PERCENT,
        "blocks": blocks.iter().map(|block| serde_json::json!({
            "path": block.path,
            "line_start": block.line_start,
            "line_end": block.line_end,
            "score": block.score,
            "semantic_relevance_score": block.semantic_relevance_score,
            "lexical_relevance_score": block.lexical_relevance_score,
            "overlap_percent": block.overlap_percent,
            "token_count": block.token_count,
            "snippet_hash": sha256_hex(block.snippet.as_bytes())
        })).collect::<Vec<_>>()
    })
    .to_string();
    let _ = connection.execute(
        "
        INSERT INTO rag_retrieval_audit (
            event_id, query_hash, trace_hash, trace_json, created_at_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
        params![
            format!("rag-retrieve-{}", unix_time_ms()),
            sha256_hex(query.as_bytes()),
            sha256_hex(trace_json.as_bytes()),
            trace_json,
            unix_time_ms()
        ],
    );
}

fn migrate_rag_audit(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS rag_retrieval_audit (
            event_id TEXT PRIMARY KEY,
            query_hash TEXT NOT NULL,
            trace_hash TEXT NOT NULL,
            trace_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_rag_retrieval_audit_created
            ON rag_retrieval_audit(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_rag_retrieval_audit_query
            ON rag_retrieval_audit(query_hash);
        ",
    )
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(project_root)
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

impl KnowledgeError {
    fn embedding(error: crate::gemma::GemmaError) -> Self {
        Self {
            code: "knowledge_embedding_failed",
            boundary: "GemmaEmbeddingRuntime",
            message: error.message,
        }
    }

    fn database(error: rusqlite::Error) -> Self {
        Self {
            code: "knowledge_database_failed",
            boundary: "KnowledgeStore",
            message: error.to_string(),
        }
    }

    fn database_boundary(error: DatabaseError) -> Self {
        Self {
            code: "knowledge_database_failed",
            boundary: "KnowledgeStore",
            message: error.to_string(),
        }
    }

    fn io(error: std::io::Error) -> Self {
        Self {
            code: "knowledge_io_failed",
            boundary: "KnowledgeStore",
            message: error.to_string(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "knowledge_invalid_request",
            boundary: "KnowledgeStore",
            message: message.into(),
        }
    }

    fn grant(message: impl Into<String>) -> Self {
        Self {
            code: "knowledge_grant_rejected",
            boundary: "KnowledgePickerGrant",
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: "knowledge_worker_failed",
            boundary: "KnowledgeStore",
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests;
