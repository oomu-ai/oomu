use super::*;
use crate::{
    browser_automation::BrowserAutomationManager,
    db::PersistenceEngine,
    foundation::digest::sha256_hex,
    sovereign_search::{self, SovereignSearchExecutionRequest},
};
use rusqlite::params;
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path},
};

pub(crate) struct SourceMaterial {
    pub evidence: SourceEvidence,
    pub content: String,
}

pub(crate) async fn read(
    source: &DelegatedSource,
    project_id: &str,
    task_run_id: &str,
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    browser: &BrowserAutomationManager,
) -> Result<SourceMaterial, String> {
    match source {
        DelegatedSource::InlineText { label, content } => {
            material("inline_text", label, content.clone(), false)
        }
        DelegatedSource::ProjectFile {
            source_id,
            relative_path,
        } => read_project_file(persistence, project_id, source_id, relative_path),
        DelegatedSource::WebSearch {
            query,
            max_results,
            authorization,
        } => {
            let response = sovereign_search::execute_sovereign_duckduckgo_search(
                SovereignSearchExecutionRequest::approved_delegation(
                    query.clone(),
                    *max_results,
                    task_run_id,
                    authorization.originating_user_objective.clone(),
                    authorization.approved_query.clone(),
                ),
                Some(app),
                Some(persistence.clone()),
            )
            .await?;
            if response.degraded && response.results.is_empty() {
                return Err(response
                    .error_code
                    .unwrap_or_else(|| "search_unavailable".into()));
            }
            material("web_search", query, response.context_json, true)
        }
        DelegatedSource::BrowserSnapshot { session_id } => {
            let snapshot = browser.read_snapshot(session_id, task_run_id)?;
            let encoded = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
            material("browser_snapshot", session_id, encoded, true)
        }
        DelegatedSource::TaskEvidence { event_types } => {
            read_task_evidence(persistence, task_run_id, event_types)
        }
    }
}

fn material(
    kind: &str,
    label: &str,
    content: String,
    observed: bool,
) -> Result<SourceMaterial, String> {
    if content.len() > 512_000 {
        return Err("Delegated source exceeds the V1 context limit.".into());
    }
    let digest = sha256_hex(content.as_bytes());
    Ok(SourceMaterial {
        evidence: SourceEvidence {
            source_ref: format!("{kind}:{}", sha256_hex(label.as_bytes())),
            source_kind: kind.into(),
            digest,
            observed,
        },
        content,
    })
}

fn read_project_file(
    engine: &PersistenceEngine,
    project_id: &str,
    source_id: &str,
    relative: &str,
) -> Result<SourceMaterial, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Delegated project file escaped its approved source.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let root:String=connection.query_row("SELECT canonical_path FROM project_sources WHERE project_id=?1 AND source_id=?2 AND grant_state='active'",params![project_id,source_id],|row|row.get(0)).map_err(|_|"Delegated Project source is unavailable or revoked.".to_string())?;
    let root = fs::canonicalize(root)
        .map_err(|_| "Delegated Project source is unavailable.".to_string())?;
    let candidate = fs::canonicalize(root.join(relative_path))
        .map_err(|_| "Delegated Project file is unavailable.".to_string())?;
    if !candidate.starts_with(&root)
        || candidate
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("Delegated Project file failed containment checks.".into());
    }
    let metadata = candidate.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 512_000 {
        return Err("Delegated Project file is not a bounded regular file.".into());
    }
    let content = fs::read_to_string(&candidate)
        .map_err(|_| "Delegated Project file must be readable UTF-8 text.".to_string())?;
    material("project_file", relative, content, true)
}

fn read_task_evidence(
    engine: &PersistenceEngine,
    task_run_id: &str,
    event_types: &[String],
) -> Result<SourceMaterial, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare("SELECT event_json FROM task_events WHERE task_run_id=?1 ORDER BY sequence")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut selected = Vec::<Value>::new();
    for raw in rows {
        let value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let event = value.get("eventType").and_then(Value::as_str).unwrap_or("");
        let class = value
            .get("evidenceClass")
            .and_then(Value::as_str)
            .unwrap_or("");
        if event_types
            .iter()
            .any(|prefix| event == prefix || event.starts_with(&format!("{prefix}.")))
            && matches!(
                class,
                "observed_result" | "verified_postcondition" | "signed_artifact"
            )
        {
            selected.push(value)
        }
    }
    if selected.is_empty() {
        return Err("No matching observed Task evidence was available to the child.".into());
    }
    material(
        "task_evidence",
        &event_types.join(","),
        serde_json::to_string(&selected).map_err(|e| e.to_string())?,
        true,
    )
}
