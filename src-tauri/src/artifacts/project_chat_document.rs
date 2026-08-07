use super::commands::create_artifact_internal;
use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::digest::sha256_hex,
    p0_contracts::{EvidenceClass, TaskId, TaskRunId},
    projects::path_scope::single_active_project_root,
    sovereign_identity::SovereignIdentity,
    tasks,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProjectChatDocumentRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub project_id: String,
    pub title: String,
    pub content: String,
    pub locale: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectChatDocumentResponse {
    pub artifact_id: String,
    pub version: u32,
}

#[tauri::command]
pub async fn create_project_chat_document(
    request: CreateProjectChatDocumentRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<CreateProjectChatDocumentResponse, String> {
    let engine = persistence.inner();
    let input = ProjectChatDocumentInput::from_request(&request)?;
    verify_project_chat_document_input(engine, &input)?;
    if let Some(response) = completed_project_chat_document(engine, &input)? {
        return Ok(response);
    }
    let (flow_id, task_run_id) = prepare_project_chat_artifact_task(engine, &input)?;
    tasks::record_domain_event(
        engine,
        &task_run_id,
        "chat_artifact.composition_verified",
        EvidenceClass::ObservedResult,
        json!({"sessionId":input.session_id,"turnId":input.turn_id,"contentSha256":sha256_hex(input.content.as_bytes())}),
    )?;
    let created = create_artifact_internal(
        CreateArtifactRequest {
            project_id: input.project_id.to_string(),
            task_run_id: task_run_id.clone(),
            document: project_chat_document(input.title, input.content, input.locale),
        },
        engine,
        identity.inner(),
    )
    .await;
    finish_project_chat_artifact_task(engine, &flow_id, &task_run_id, created)
}

struct ProjectChatDocumentInput<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    generation_token: &'a str,
    project_id: &'a str,
    title: &'a str,
    content: &'a str,
    locale: &'a str,
}

impl<'a> ProjectChatDocumentInput<'a> {
    fn from_request(request: &'a CreateProjectChatDocumentRequest) -> Result<Self, String> {
        let input = Self {
            session_id: request.session_id.trim(),
            turn_id: request.turn_id.trim(),
            generation_token: request.generation_token.trim(),
            project_id: request.project_id.trim(),
            title: request.title.trim(),
            content: request.content.trim(),
            locale: request.locale.trim(),
        };
        if input.session_id.is_empty()
            || input.turn_id.is_empty()
            || input.generation_token.is_empty()
            || input.project_id.is_empty()
            || input.title.is_empty()
            || input.title.chars().count() > 240
            || input.content.is_empty()
            || input.content.chars().count() > 240_000
            || !(2..=35).contains(&input.locale.len())
        {
            return Err("The document request is incomplete.".to_string());
        }
        Ok(input)
    }
}

fn verify_project_chat_document_input(
    engine: &PersistenceEngine,
    input: &ProjectChatDocumentInput<'_>,
) -> Result<(), String> {
    let session = engine
        .select_chat_session_by_id(input.session_id)
        .map_err(|_| "This chat is no longer available.".to_string())?;
    if session.project_id.as_deref() != Some(input.project_id) {
        return Err("This chat is no longer connected to that Project.".to_string());
    }
    single_active_project_root(engine, input.project_id)?;
    let persisted_content = engine.open_connection().map_err(|error| error.to_string())?.query_row(
        "SELECT content FROM chat_messages WHERE workspace_id=?1 AND session_id=?2 AND role='assistant'
         AND COALESCE(is_compacted,0)=0 AND json_extract(metadata_json,'$.turnId')=?3
         AND json_extract(metadata_json,'$.generationToken')=?4
         ORDER BY timestamp_ms DESC,id DESC LIMIT 1",
        params![session.workspace_id, input.session_id, input.turn_id, input.generation_token],
        |row| row.get::<_, String>(0),
    ).optional().map_err(|error| error.to_string())?
        .ok_or_else(|| "OOMU could not verify the finished Project response.".to_string())?;
    if persisted_content.trim() != input.content {
        return Err("The finished Project response changed before document creation.".to_string());
    }
    Ok(())
}

fn completed_project_chat_document(
    engine: &PersistenceEngine,
    input: &ProjectChatDocumentInput<'_>,
) -> Result<Option<CreateProjectChatDocumentResponse>, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT records.artifact_id,records.current_version FROM task_runs tasks
         JOIN artifact_records records ON records.task_run_id=tasks.task_run_id
         JOIN artifact_versions versions ON versions.artifact_id=records.artifact_id
          AND versions.version=records.current_version
         WHERE tasks.runtime_kind='taskflow' AND tasks.runtime_record_id=?1 AND tasks.project_id=?2
          AND tasks.state='completed' AND versions.status='verified'
         ORDER BY records.updated_at_ms DESC LIMIT 1",
            params![format!("chat-artifact-{}", input.turn_id), input.project_id],
            |row| {
                Ok(CreateProjectChatDocumentResponse {
                    artifact_id: row.get(0)?,
                    version: row.get::<_, i64>(1)? as u32,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn prepare_project_chat_artifact_task(
    engine: &PersistenceEngine,
    input: &ProjectChatDocumentInput<'_>,
) -> Result<(String, String), String> {
    let flow_id = format!("chat-artifact-{}", input.turn_id);
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let existing_task = connection
        .query_row(
            "SELECT task_run_id FROM task_runs WHERE runtime_kind='taskflow'
         AND runtime_record_id=?1 AND project_id=?2",
            params![flow_id, input.project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let task_run_id = existing_task
        .clone()
        .unwrap_or_else(|| TaskRunId::new().to_string());
    let now = crate::foundation::clock::unix_time_ms_i64();
    if existing_task.is_some() {
        connection.execute("UPDATE taskflows SET status='active',updated_at_ms=?2 WHERE flow_id=?1 AND status IN ('failed','cancelled')", params![flow_id, now]).map_err(|error| error.to_string())?;
        connection.execute("UPDATE task_runs SET state='running',last_error=NULL,completed_at_ms=NULL,updated_at_ms=?2,recovery_state='reconciled' WHERE task_run_id=?1 AND state IN ('failed','cancelled')", params![task_run_id, now]).map_err(|error| error.to_string())?;
        return Ok((flow_id, task_run_id));
    }
    let task_id = TaskId::new().to_string();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT INTO taskflows (flow_id,mission_id,parent_session_id,directive,status,created_at_ms,
         updated_at_ms,chat_turn_id,chat_generation_token,chat_session_id)
         VALUES (?1,?1,?2,?3,'active',?4,?4,?5,?6,?2)",
        params![flow_id, input.session_id, input.title, now, input.turn_id, input.generation_token],
    ).map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,
         origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state)
         VALUES (?1,?2,?3,'taskflow',?4,'running','chat',?2,?5,?6,?6,'reconciled')",
        params![task_run_id, task_id, input.project_id, flow_id, input.title, now],
    ).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok((flow_id, task_run_id))
}

fn finish_project_chat_artifact_task(
    engine: &PersistenceEngine,
    flow_id: &str,
    task_run_id: &str,
    created: Result<ArtifactRecord, String>,
) -> Result<CreateProjectChatDocumentResponse, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    match created {
        Ok(record) => {
            connection
                .execute(
                    "UPDATE taskflows SET status='verified',updated_at_ms=?2 WHERE flow_id=?1",
                    params![flow_id, now],
                )
                .map_err(|error| error.to_string())?;
            connection.execute("UPDATE task_runs SET state='completed',updated_at_ms=?2,completed_at_ms=?2,last_error=NULL WHERE task_run_id=?1 AND state='running'", params![task_run_id, now]).map_err(|error| error.to_string())?;
            Ok(CreateProjectChatDocumentResponse {
                artifact_id: record.artifact_id,
                version: record.current_version,
            })
        }
        Err(error) => {
            connection
                .execute(
                    "UPDATE taskflows SET status='failed',updated_at_ms=?2 WHERE flow_id=?1",
                    params![flow_id, now],
                )
                .map_err(|database_error| database_error.to_string())?;
            connection.execute("UPDATE task_runs SET state='failed',updated_at_ms=?2,completed_at_ms=?2,last_error=?3 WHERE task_run_id=?1 AND state='running'", params![task_run_id, now, error.as_str()]).map_err(|database_error| database_error.to_string())?;
            Err(error)
        }
    }
}

fn project_chat_document(title: &str, content: &str, locale: &str) -> ArtifactDocument {
    let sections = markdown_artifact_sections(title, content);
    ArtifactDocument {
        schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
        metadata: ArtifactMetadata {
            title: title.to_string(),
            subtitle: String::new(),
            author: "OOMU".to_string(),
            subject: title.to_string(),
            keywords: vec!["Project".to_string()],
            language: locale.to_string(),
        },
        theme: ThemeTokens::default(),
        page: PageControls::default(),
        header: Some(title.to_string()),
        footer: Some("OOMU".to_string()),
        sections,
    }
}

fn markdown_artifact_sections(title: &str, content: &str) -> Vec<ArtifactSection> {
    let mut sections = Vec::new();
    let mut heading = title.to_string();
    let mut blocks = Vec::new();
    let mut paragraph = Vec::new();
    let mut list_items = Vec::new();

    let flush_paragraph = |paragraph: &mut Vec<String>, blocks: &mut Vec<ArtifactBlock>| {
        if !paragraph.is_empty() {
            blocks.push(ArtifactBlock::Paragraph {
                text: paragraph.join(" "),
                style: ParagraphStyle::Body,
                factual: false,
                sources: Vec::new(),
            });
            paragraph.clear();
        }
    };
    let flush_list = |items: &mut Vec<String>, blocks: &mut Vec<ArtifactBlock>| {
        if !items.is_empty() {
            blocks.push(ArtifactBlock::List {
                ordered: false,
                items: std::mem::take(items),
                factual: false,
                sources: Vec::new(),
            });
        }
    };
    let flush_section =
        |heading: &str, blocks: &mut Vec<ArtifactBlock>, sections: &mut Vec<ArtifactSection>| {
            if !blocks.is_empty() {
                sections.push(ArtifactSection {
                    heading: heading.to_string(),
                    page_break_before: false,
                    blocks: std::mem::take(blocks),
                });
            }
        };

    let lines = content.lines().map(str::trim).collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if let Some((table, consumed)) = markdown_table(&lines, index) {
            flush_paragraph(&mut paragraph, &mut blocks);
            flush_list(&mut list_items, &mut blocks);
            blocks.push(table);
            index += consumed;
            continue;
        }
        if let Some(next_heading) = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "))
            .or_else(|| line.strip_prefix("# "))
        {
            flush_paragraph(&mut paragraph, &mut blocks);
            flush_list(&mut list_items, &mut blocks);
            flush_section(&heading, &mut blocks, &mut sections);
            heading = next_heading.trim().to_string();
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_items.push(item.trim().to_string());
        } else if line.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            flush_list(&mut list_items, &mut blocks);
        } else {
            flush_list(&mut list_items, &mut blocks);
            paragraph.push(line.to_string());
        }
        index += 1;
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    flush_list(&mut list_items, &mut blocks);
    flush_section(&heading, &mut blocks, &mut sections);
    if sections.is_empty() {
        sections.push(ArtifactSection {
            heading: title.to_string(),
            page_break_before: false,
            blocks: vec![ArtifactBlock::Paragraph {
                text: content.to_string(),
                style: ParagraphStyle::Body,
                factual: false,
                sources: Vec::new(),
            }],
        });
    }
    sections
}

fn markdown_table(lines: &[&str], start: usize) -> Option<(ArtifactBlock, usize)> {
    if start + 2 >= lines.len() {
        return None;
    }
    let headers = markdown_table_row(lines[start])?;
    if headers.is_empty() || headers.len() > 12 {
        return None;
    }
    let separator = markdown_table_row(lines[start + 1])?;
    if separator.len() != headers.len()
        || separator.iter().any(|cell| {
            let marker = cell.trim().trim_matches(':');
            marker.len() < 3 || !marker.chars().all(|character| character == '-')
        })
    {
        return None;
    }
    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() && rows.len() < 30 {
        let Some(row) = markdown_table_row(lines[index]) else {
            break;
        };
        if row.len() != headers.len() {
            break;
        }
        rows.push(row);
        index += 1;
    }
    if rows.is_empty() {
        return None;
    }
    Some((
        ArtifactBlock::Table {
            headers,
            rows,
            caption: String::new(),
            factual: false,
            sources: Vec::new(),
        },
        index - start,
    ))
}

fn markdown_table_row(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    Some(
        line.trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect(),
    )
}

#[cfg(test)]
mod project_chat_document_tests {
    use super::*;

    #[test]
    fn keeps_markdown_results_tables_as_document_tables() {
        let sections = markdown_artifact_sections(
            "Quarterly update",
            "# Quarterly update\n\n| Outcome | Result |\n| --- | --- |\n| Served | 42 |",
        );
        assert!(sections
            .iter()
            .flat_map(|section| &section.blocks)
            .any(|block| {
                matches!(block, ArtifactBlock::Table { headers, rows, .. }
                if headers == &["Outcome", "Result"]
                    && rows == &[vec!["Served".to_string(), "42".to_string()]])
            }));
    }
}
