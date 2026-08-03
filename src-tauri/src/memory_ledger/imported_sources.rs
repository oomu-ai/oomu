use super::{
    guard_memory_text, memory_relevance, parse_daily_journal_file, sha256_hex, AgentMemoryEntry,
    ImportedAgentMemoryCard, JournalImportFile, MemoryLedger, MemoryLedgerError,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const PARSER_VERSION: &str = "daily-journal-v2";
const MAX_ACTIVE_SOURCES_PER_AGENT: i64 = 512;
const MAX_RETAINED_SOURCE_VERSIONS_PER_AGENT: i64 = 768;

pub(crate) fn memory_limit_for_context_budget(context_budget_tokens: usize) -> usize {
    (context_budget_tokens / 768).clamp(3, 500)
}

#[derive(Clone, Debug)]
pub(super) struct ImportedSourceRecord {
    pub relative_path: String,
    pub content_digest: String,
    pub modified_at_ms: Option<i64>,
    pub cards: Vec<ImportedAgentMemoryCard>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadImportedSourceRequest {
    pub agent_id: String,
    #[serde(default)]
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSourceReceipt {
    pub agent_id: String,
    pub relative_path: String,
    pub content_digest: String,
    pub parser_version: String,
    pub modified_at_ms: Option<i64>,
    pub newest_entry_date: Option<String>,
    pub indexed_at_ms: i64,
    pub entries: Vec<String>,
    pub receipt_digest: String,
}

pub(super) fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS imported_memory_sources (
            source_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            content_digest TEXT NOT NULL,
            parser_version TEXT NOT NULL,
            modified_at_ms INTEGER,
            newest_entry_date TEXT,
            indexed_at_ms INTEGER NOT NULL,
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
            UNIQUE(agent_id, relative_path, content_digest, parser_version)
        );
        CREATE INDEX IF NOT EXISTS idx_imported_sources_active
            ON imported_memory_sources(agent_id, active, newest_entry_date DESC, indexed_at_ms DESC);
        CREATE TABLE IF NOT EXISTS imported_memory_source_entries (
            source_id TEXT NOT NULL,
            memory_entry_id INTEGER NOT NULL,
            ordinal INTEGER NOT NULL,
            entry_digest TEXT NOT NULL,
            entry_date TEXT,
            PRIMARY KEY(source_id, memory_entry_id),
            FOREIGN KEY(source_id) REFERENCES imported_memory_sources(source_id) ON DELETE CASCADE,
            FOREIGN KEY(memory_entry_id) REFERENCES agent_memory_entries(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_imported_source_entries_memory
            ON imported_memory_source_entries(memory_entry_id, source_id);
        ",
    )
}

pub(super) fn content_digest(file: &JournalImportFile) -> String {
    sha256_hex(file.content.as_bytes())
}

pub(super) fn prepare_changed_sources(
    connection: &Connection,
    agent_id: &str,
    mut cards: Vec<ImportedAgentMemoryCard>,
    journal_files: Vec<JournalImportFile>,
) -> Result<(Vec<ImportedAgentMemoryCard>, Vec<ImportedSourceRecord>), MemoryLedgerError> {
    let mut sources = Vec::new();
    for journal_file in journal_files {
        let content_digest = content_digest(&journal_file);
        if source_is_current(
            connection,
            agent_id,
            &journal_file.relative_path,
            &content_digest,
        )
        .map_err(MemoryLedgerError::database)?
        {
            continue;
        }
        let source_cards = parse_daily_journal_file(&journal_file)?;
        cards.extend(source_cards.clone());
        sources.push(ImportedSourceRecord {
            relative_path: journal_file.relative_path,
            content_digest,
            modified_at_ms: journal_file.modified_at_ms,
            cards: source_cards,
        });
    }
    Ok((cards, sources))
}

pub(super) fn source_is_current(
    connection: &Connection,
    agent_id: &str,
    relative_path: &str,
    content_digest: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM imported_memory_sources
            WHERE agent_id=?1 AND relative_path=?2 AND content_digest=?3
              AND parser_version=?4 AND active=1
        )",
        params![
            agent_id,
            normalize_path(relative_path),
            content_digest,
            PARSER_VERSION
        ],
        |row| row.get(0),
    )
}

pub(super) fn record_sources(
    transaction: &Transaction<'_>,
    agent_id: &str,
    sources: &[ImportedSourceRecord],
    indexed_at_ms: i64,
) -> rusqlite::Result<()> {
    for source in sources {
        let relative_path = normalize_path(&source.relative_path);
        let source_id = sha256_hex(
            format!(
                "{agent_id}\n{relative_path}\n{}\n{PARSER_VERSION}",
                source.content_digest
            )
            .as_bytes(),
        );
        let newest_entry_date = source
            .cards
            .iter()
            .filter_map(|card| card.scope.strip_prefix("journal:"))
            .filter(|date| *date != "undated")
            .max()
            .map(str::to_string);

        transaction.execute(
            "UPDATE imported_memory_sources SET active=0
             WHERE agent_id=?1 AND relative_path=?2 AND source_id!=?3 AND active=1",
            params![agent_id, relative_path, source_id],
        )?;
        transaction.execute(
            "INSERT INTO imported_memory_sources (
                source_id,agent_id,relative_path,content_digest,parser_version,
                modified_at_ms,newest_entry_date,indexed_at_ms,active
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1)
             ON CONFLICT(source_id) DO UPDATE SET
                modified_at_ms=excluded.modified_at_ms,
                newest_entry_date=excluded.newest_entry_date,
                indexed_at_ms=excluded.indexed_at_ms,
                active=1",
            params![
                source_id,
                agent_id,
                relative_path,
                source.content_digest,
                PARSER_VERSION,
                source.modified_at_ms,
                newest_entry_date,
                indexed_at_ms,
            ],
        )?;
        transaction.execute(
            "DELETE FROM imported_memory_source_entries WHERE source_id=?1",
            params![source_id],
        )?;
        for (ordinal, card) in source.cards.iter().enumerate() {
            let memory_entry_id: Option<i64> = transaction
                .query_row(
                    "SELECT id FROM agent_memory_entries
                     WHERE agent_id=?1 AND memory_kind=?2 AND scope=?3 AND content=?4
                     LIMIT 1",
                    params![agent_id, card.memory_kind, card.scope, card.content],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(memory_entry_id) = memory_entry_id else {
                continue;
            };
            let entry_date = card
                .scope
                .strip_prefix("journal:")
                .filter(|date| *date != "undated");
            transaction.execute(
                "INSERT OR REPLACE INTO imported_memory_source_entries (
                    source_id,memory_entry_id,ordinal,entry_digest,entry_date
                 ) VALUES (?1,?2,?3,?4,?5)",
                params![
                    source_id,
                    memory_entry_id,
                    ordinal as i64,
                    sha256_hex(card.content.as_bytes()),
                    entry_date,
                ],
            )?;
        }
    }
    enforce_retention(transaction, agent_id)
}

fn enforce_retention(transaction: &Transaction<'_>, agent_id: &str) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE imported_memory_sources SET active=0 WHERE source_id IN (
            SELECT source_id FROM imported_memory_sources
            WHERE agent_id=?1 AND active=1
            ORDER BY COALESCE(newest_entry_date,'' ) DESC, indexed_at_ms DESC
            LIMIT -1 OFFSET ?2
        )",
        params![agent_id, MAX_ACTIVE_SOURCES_PER_AGENT],
    )?;
    transaction.execute(
        "DELETE FROM imported_memory_sources WHERE source_id IN (
            SELECT source_id FROM imported_memory_sources
            WHERE agent_id=?1
            ORDER BY active DESC, indexed_at_ms DESC
            LIMIT -1 OFFSET ?2
        )",
        params![agent_id, MAX_RETAINED_SOURCE_VERSIONS_PER_AGENT],
    )?;
    Ok(())
}

pub(super) fn recall_sql() -> &'static str {
    "SELECT id, agent_id, memory_kind, scope, content, confidence, source_session,
            source_turn, contradicted_by, visibility, signature_json, created_at_ms,
            last_confirmed_at_ms
     FROM agent_memory_entries
     WHERE agent_id = ?1 AND contradicted_by IS NULL
       AND (project_id IS NULL OR project_id = ?3)
       AND (
        agent_memory_entries.memory_kind!='daily_journal'
        OR agent_memory_entries.source_session NOT LIKE 'journal_import:%'
        OR NOT EXISTS (
            SELECT 1 FROM imported_memory_source_entries mapped
            WHERE mapped.memory_entry_id=agent_memory_entries.id
        )
        OR EXISTS (
            SELECT 1 FROM imported_memory_source_entries mapped
            JOIN imported_memory_sources source ON source.source_id=mapped.source_id
            WHERE mapped.memory_entry_id=agent_memory_entries.id AND source.active=1
        )
       )
     ORDER BY last_confirmed_at_ms DESC, created_at_ms DESC
     LIMIT ?2"
}

pub(super) fn journal_date(memory: &AgentMemoryEntry) -> Option<&str> {
    (memory.memory_kind == "daily_journal")
        .then(|| memory.scope.strip_prefix("journal:"))
        .flatten()
        .filter(|value| *value != "undated")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChronologyPreference {
    Relevance,
    OldestFirst,
    NewestFirst,
}

pub(super) fn chronology_preference(query: &str) -> ChronologyPreference {
    let query = query.to_ascii_lowercase();
    if ["chronological", "chronology", "timeline", "oldest first"]
        .iter()
        .any(|marker| query.contains(marker))
    {
        ChronologyPreference::OldestFirst
    } else if ["latest", "newest", "most recent", "recent"]
        .iter()
        .any(|marker| query.contains(marker))
    {
        ChronologyPreference::NewestFirst
    } else {
        ChronologyPreference::Relevance
    }
}

pub(super) fn compare_memories(
    left: &AgentMemoryEntry,
    right: &AgentMemoryEntry,
    query_terms: &[String],
    chronology: ChronologyPreference,
) -> std::cmp::Ordering {
    let chronological = match chronology {
        ChronologyPreference::OldestFirst => journal_date(left).cmp(&journal_date(right)),
        ChronologyPreference::NewestFirst => journal_date(right).cmp(&journal_date(left)),
        ChronologyPreference::Relevance => std::cmp::Ordering::Equal,
    };
    if chronological != std::cmp::Ordering::Equal {
        return chronological;
    }
    memory_relevance(right, query_terms)
        .partial_cmp(&memory_relevance(left, query_terms))
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| journal_date(right).cmp(&journal_date(left)))
}

pub(super) fn read_receipt(
    connection: &Connection,
    request: &ReadImportedSourceRequest,
) -> rusqlite::Result<Option<ImportedSourceReceipt>> {
    let relative_path = request.relative_path.as_deref().map(normalize_path);
    let mut statement = connection.prepare(
        "SELECT source_id,relative_path,content_digest,parser_version,modified_at_ms,
                newest_entry_date,indexed_at_ms
         FROM imported_memory_sources
         WHERE agent_id=?1 AND active=1 AND (?2 IS NULL OR relative_path=?2)
         ORDER BY COALESCE(newest_entry_date,'') DESC,indexed_at_ms DESC LIMIT 1",
    )?;
    let source = statement
        .query_row(params![request.agent_id, relative_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .optional()?;
    let Some((
        source_id,
        relative_path,
        content_digest,
        parser_version,
        modified_at_ms,
        newest_entry_date,
        indexed_at_ms,
    )) = source
    else {
        return Ok(None);
    };
    let mut entries_statement = connection.prepare(
        "SELECT memory.content FROM imported_memory_source_entries mapped
         JOIN agent_memory_entries memory ON memory.id=mapped.memory_entry_id
         WHERE mapped.source_id=?1 ORDER BY mapped.ordinal",
    )?;
    let entries = entries_statement
        .query_map(params![source_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let receipt_digest = sha256_hex(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            request.agent_id,
            relative_path,
            content_digest,
            parser_version,
            entries.join("\n---\n")
        )
        .as_bytes(),
    );
    Ok(Some(ImportedSourceReceipt {
        agent_id: request.agent_id.clone(),
        relative_path,
        content_digest,
        parser_version,
        modified_at_ms,
        newest_entry_date,
        indexed_at_ms,
        entries,
        receipt_digest,
    }))
}

pub async fn read_imported_agent_source(
    request: ReadImportedSourceRequest,
    ledger: tauri::State<'_, MemoryLedger>,
) -> Result<ImportedSourceReceipt, MemoryLedgerError> {
    let request = ReadImportedSourceRequest {
        agent_id: guard_memory_text("agent_id", &request.agent_id)?,
        relative_path: request
            .relative_path
            .map(|path| guard_memory_text("relative_path", &path))
            .transpose()?,
    };
    let ledger = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = ledger
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        read_receipt(&connection, &request)
            .map_err(MemoryLedgerError::database)?
            .ok_or_else(|| MemoryLedgerError::invalid("Imported source was not found."))
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sovereign_identity::SovereignIdentity;
    use std::{env, fs};

    fn card(date: &str, path: &str, entry: &str) -> ImportedAgentMemoryCard {
        ImportedAgentMemoryCard {
            memory_kind: "daily_journal".to_string(),
            scope: format!("journal:{date}"),
            content: format!("Journal date: {date}\nSource file: {path}\nEntry: {entry}"),
            confidence: 0.86,
            source_session: format!("journal_import:{path}"),
            visibility: "private".to_string(),
        }
    }

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE agent_memory_entries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL,
                    memory_kind TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    content TEXT NOT NULL,
                    source_session TEXT NOT NULL
                 );",
            )
            .unwrap();
        migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn changed_source_supersedes_old_entries_and_produces_a_real_receipt() {
        let mut connection = connection();
        let first = card("2026-06-01", "journal/2026-06-01.md", "Amber with Mira");
        connection.execute(
            "INSERT INTO agent_memory_entries (agent_id,memory_kind,scope,content,source_session)
             VALUES ('agent','daily_journal',?1,?2,?3)",
            params![first.scope, first.content, first.source_session],
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        record_sources(
            &transaction,
            "agent",
            &[ImportedSourceRecord {
                relative_path: "journal/2026-06-01.md".to_string(),
                content_digest: sha256_hex(b"first"),
                modified_at_ms: Some(1),
                cards: vec![first],
            }],
            10,
        )
        .unwrap();
        transaction.commit().unwrap();

        let latest = card("2026-06-03", "journal/2026-06-01.md", "Blue with Omar");
        connection.execute(
            "INSERT INTO agent_memory_entries (agent_id,memory_kind,scope,content,source_session)
             VALUES ('agent','daily_journal',?1,?2,?3)",
            params![latest.scope, latest.content, latest.source_session],
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        record_sources(
            &transaction,
            "agent",
            &[ImportedSourceRecord {
                relative_path: "journal/2026-06-01.md".to_string(),
                content_digest: sha256_hex(b"second"),
                modified_at_ms: Some(2),
                cards: vec![latest],
            }],
            20,
        )
        .unwrap();
        transaction.commit().unwrap();

        assert!(!source_is_current(
            &connection,
            "agent",
            "journal/2026-06-01.md",
            &sha256_hex(b"first"),
        )
        .unwrap());
        let receipt = read_receipt(
            &connection,
            &ReadImportedSourceRequest {
                agent_id: "agent".to_string(),
                relative_path: Some("journal/2026-06-01.md".to_string()),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(receipt.newest_entry_date.as_deref(), Some("2026-06-03"));
        assert!(receipt.entries.join("\n").contains("Blue with Omar"));
        assert!(!receipt.receipt_digest.is_empty());
    }

    #[test]
    fn retrieval_intent_distinguishes_latest_from_chronological() {
        assert_eq!(
            chronology_preference("What happened most recently?"),
            ChronologyPreference::NewestFirst
        );
        assert_eq!(
            chronology_preference("List these in chronological order"),
            ChronologyPreference::OldestFirst
        );
        assert_eq!(
            chronology_preference("What did Mira decide?"),
            ChronologyPreference::Relevance
        );
    }

    #[test]
    fn refresh_makes_a_newer_imported_file_win_the_same_latest_query() {
        let root = env::temp_dir().join(format!(
            "oomu-imported-memory-refresh-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let ledger = MemoryLedger::initialize_at(root.join("memory.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let agent_id = "agent-refresh-test";
        let query = "Looking only at the imported UX Memory Fixture entries, what is the latest release color and who owns it? Include the source date and filename.";

        ledger
            .import_agent_memory_cards_sync(
                agent_id,
                Vec::new(),
                vec![
                    JournalImportFile {
                        relative_path: "2026-06-01.md".to_string(),
                        extension: "md".to_string(),
                        content: "# 2026-06-01\n- UX Memory Fixture release color is Amber, owned by Mira.".to_string(),
                        modified_at_ms: Some(1),
                    },
                    JournalImportFile {
                        relative_path: "2026-06-02.md".to_string(),
                        extension: "md".to_string(),
                        content: "# 2026-06-02\n- UX Memory Fixture release color is Green, owned by Nia.".to_string(),
                        modified_at_ms: Some(2),
                    },
                ],
                &identity,
            )
            .unwrap();
        let before = ledger
            .select_agent_memories_sync(agent_id, query, 3, None, &identity)
            .unwrap();
        assert!(before[0].content.contains("Green, owned by Nia"));
        assert!(before[0].content.contains("2026-06-02.md"));

        ledger
            .import_agent_memory_cards_sync(
                agent_id,
                Vec::new(),
                vec![JournalImportFile {
                    relative_path: "2026-06-03.md".to_string(),
                    extension: "md".to_string(),
                    content:
                        "# 2026-06-03\n- UX Memory Fixture release color is Blue, owned by Omar."
                            .to_string(),
                    modified_at_ms: Some(3),
                }],
                &identity,
            )
            .unwrap();
        let after = ledger
            .select_agent_memories_sync(agent_id, query, 3, None, &identity)
            .unwrap();
        assert!(after[0].content.contains("Blue, owned by Omar"));
        assert!(after[0].content.contains("2026-06-03.md"));

        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }
}
