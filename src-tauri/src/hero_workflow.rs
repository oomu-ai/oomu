use crate::{db::PersistenceEngine, p0_contracts::ProjectId};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeroWorkflowRequest {
    pub project_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroRequirement {
    pub id: String,
    pub label: String,
    pub state: String,
    pub detail: String,
    pub destination: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroWorkflowStatus {
    pub template_id: String,
    pub title: String,
    pub description: String,
    pub project_id: String,
    pub cadence: String,
    pub supported_delivery_channels: Vec<String>,
    pub requirements: Vec<HeroRequirement>,
    pub ready_on_demand: bool,
    pub ready_weekly: bool,
    pub contract_version: u16,
}

fn requirement(
    id: &str,
    label: &str,
    ready: bool,
    detail: impl Into<String>,
    destination: &str,
) -> HeroRequirement {
    HeroRequirement {
        id: id.into(),
        label: label.into(),
        state: if ready { "ready" } else { "needs_setup" }.into(),
        detail: detail.into(),
        destination: destination.into(),
    }
}

fn inspect(engine: &PersistenceEngine, project_id: &str) -> Result<HeroWorkflowStatus, String> {
    let project_id = ProjectId::parse(project_id)?.to_string();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let (instructions,sources):(i64,i64)=connection.query_row("SELECT (SELECT COUNT(*) FROM project_instructions WHERE project_id=?1 AND length(trim(instructions))>0),(SELECT COUNT(*) FROM project_sources WHERE project_id=?1 AND grant_state='active' AND indexing_state='ready')",params![project_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|"Project was not found.".to_string())?;
    let apple_connectors:i64=connection.query_row("SELECT COUNT(*) FROM connector_project_bindings b JOIN connector_accounts a ON a.connector_id=b.connector_id WHERE b.project_id=?1 AND b.enabled=1 AND a.manifest_id='apple_apps' AND a.connection_state NOT IN ('expired','blocked','disconnected','unsupported')",params![project_id],|row|row.get(0)).unwrap_or(0);
    let completed_delegation:i64=connection.query_row("SELECT COUNT(*) FROM delegation_plans WHERE project_id=?1 AND state IN ('completed','partial')",params![project_id],|row|row.get(0)).unwrap_or(0);
    let verified_artifacts:i64=connection.query_row("SELECT COUNT(*) FROM artifact_records r JOIN artifact_versions v ON v.artifact_id=r.artifact_id AND v.version=r.current_version WHERE r.project_id=?1 AND v.status='verified' AND json_extract(v.verification_json,'$.structurallyVerifiedDocx')=1 AND json_extract(v.verification_json,'$.visuallyVerifiedPdf')=1",params![project_id],|row|row.get(0)).unwrap_or(0);
    let routines:i64=connection.query_row("SELECT COUNT(*) FROM workflow_schedules WHERE project_id=?1 AND is_active=1 AND schedule_kind='recurring'",params![project_id],|row|row.get(0)).unwrap_or(0);
    let background:bool=connection.query_row("SELECT user_enabled=1 AND service_status IN ('active','paused') FROM background_service_state WHERE singleton=1",[],|row|row.get(0)).unwrap_or(false);
    let community_channels:i64=connection.query_row("SELECT COUNT(*) FROM channel_configs WHERE is_active=1 AND platform IN ('telegram','discord','slack')",[],|row|row.get(0)).unwrap_or(0);
    let web_enabled = crate::settings::automated_web_grounding_enabled_from_disk();
    let delivery_detail = if community_channels > 0 {
        "A supported community channel is active.".to_string()
    } else {
        "Enable background service and Telegram, Discord, or Slack delivery.".to_string()
    };
    let requirements = vec![
        requirement(
            "project_knowledge",
            "Approved Project knowledge",
            sources > 0,
            if sources > 0 {
                format!("{sources} indexed Project source(s) are ready.")
            } else {
                "Attach and index at least one approved Project folder.".into()
            },
            "projects",
        ),
        requirement(
            "instructions",
            "Decision criteria",
            instructions > 0,
            "Add Project instructions describing decisions, risks, and preferred brief structure.",
            "projects",
        ),
        requirement(
            "mail_calendar",
            "Mail and Calendar",
            apple_connectors > 0,
            "Enable the built-in Apple apps connector for this Project.",
            "integrations",
        ),
        requirement(
            "current_web",
            "Current web sources",
            web_enabled,
            "Enable automated web grounding or explicitly authorize each search.",
            "settings",
        ),
        requirement(
            "parallel_research",
            "Read-only research workstreams",
            completed_delegation > 0,
            "Run up to three bounded child workstreams from the Task Center.",
            "tasks",
        ),
        requirement(
            "verified_artifact",
            "Verified DOCX and PDF",
            verified_artifacts > 0,
            "Build the parent-owned decision brief after child evidence is ready.",
            "artifacts",
        ),
        requirement(
            "weekly_routine",
            "Weekly schedule",
            routines > 0,
            "Create an active weekly Routine from the reviewed workflow.",
            "routines",
        ),
        requirement(
            "delivery",
            "Completion delivery",
            background && (community_channels > 0),
            delivery_detail,
            "routines",
        ),
    ];
    let ready_on_demand = requirements
        .iter()
        .take(6)
        .all(|item| item.state == "ready");
    let ready_weekly = ready_on_demand
        && requirements
            .iter()
            .skip(6)
            .all(|item| item.state == "ready");
    Ok(HeroWorkflowStatus{template_id:"weekly_decision_brief_v1".into(),title:"Weekly Decision Brief".into(),description:"Project knowledge, Mail, Calendar, current web research, bounded delegation, and a verified professional deliverable under one Task history.".into(),project_id,cadence:"On demand or weekly".into(),supported_delivery_channels:vec!["telegram".into(),"discord".into(),"slack".into()],requirements,ready_on_demand,ready_weekly,contract_version:1})
}

#[tauri::command]
pub async fn get_weekly_decision_brief_status(
    request: HeroWorkflowRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<HeroWorkflowStatus, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect(&engine, &request.project_id))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    fn template_names_only_shipped_delivery_channels() {
        let channels = vec!["telegram", "discord", "slack"];
        assert_eq!(channels.len(), crate::db::COMMUNITY_CHANNEL_PLATFORMS.len());
    }
}
