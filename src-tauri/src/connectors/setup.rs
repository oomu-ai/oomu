use super::{
    repository, RunSetupSampleRequest, SaveSetupProgressRequest, SetupCommandError, SetupState,
};
use crate::{
    agent_manager::AgentManager,
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    gemma::{format_gemma4_chat_prompt, GemmaService, InferRequest},
    inference::{self, InferenceMessage, InferenceRequest},
    projects::{CreateProjectRequest, ProjectDataPolicy},
    settings,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use tauri_plugin_notification::{NotificationExt, PermissionState};

#[derive(Clone, Debug)]
struct SetupCopy {
    project_name: String,
    project_description: String,
    task_directive: String,
    model_system_prompt: String,
    model_prompt: String,
    notification_title: String,
    notification_body: String,
}

impl SetupCopy {
    fn load(engine: &PersistenceEngine) -> Result<Self, SetupCommandError> {
        let locale =
            settings::locale_state_for_engine(engine, None).map_err(SetupCommandError::internal)?;
        Self::from_translations(&locale.translations)
    }

    fn from_translations(translations: &Value) -> Result<Self, SetupCommandError> {
        let text = |key| setup_copy_value(translations, key);
        Ok(Self {
            project_name: text("sample_name")?,
            project_description: text("sample_project_description")?,
            task_directive: text("sample_task_directive")?,
            model_system_prompt: text("sample_model_system")?,
            model_prompt: text("sample_model_prompt")?,
            notification_title: text("notification_title")?,
            notification_body: text("notification_body")?,
        })
    }
}

fn setup_copy_value(translations: &Value, key: &str) -> Result<String, SetupCommandError> {
    translations
        .get("setup")
        .and_then(|setup| setup.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| SetupCommandError::internal(format!("Missing locale key setup.{key}")))
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| error.to_string())?
}

fn setup_local_infer_request(sample_id: &str, attempt: usize, copy: &SetupCopy) -> InferRequest {
    let prompt = format_gemma4_chat_prompt(
        &copy.model_system_prompt,
        &[("user".to_string(), copy.model_prompt.clone())],
    );
    let mut request = InferRequest::new(prompt);
    request.session_id = Some(format!("{sample_id}-attempt-{attempt}"));
    request.prompt_is_full_context = true;
    request.deterministic = true;
    request.max_tokens = Some(80);
    request
}

fn run_local_setup_sample(
    service: &GemmaService,
    sample_id: &str,
    copy: &SetupCopy,
) -> Result<String, SetupCommandError> {
    for attempt in 1..=3 {
        let response = service
            .infer_sync(setup_local_infer_request(sample_id, attempt, copy))
            .map_err(|error| {
                SetupCommandError::operational("setup_model_execution_failed", error.message)
            })?;
        let output = response.text.trim();
        if !output.is_empty() {
            return Ok(output.to_string());
        }
    }
    Err(SetupCommandError::new("setup_model_output_empty"))
}

fn request_setup_notification_permission(app: &tauri::AppHandle) -> Result<(), SetupCommandError> {
    let notification = app.notification();
    let permission = match notification.permission_state() {
        Ok(PermissionState::Granted) => PermissionState::Granted,
        Ok(PermissionState::Prompt | PermissionState::PromptWithRationale) => {
            notification.request_permission().map_err(|error| {
                SetupCommandError::operational(
                    "setup_notification_permission_request_failed",
                    error,
                )
            })?
        }
        Ok(PermissionState::Denied) => PermissionState::Denied,
        Err(error) => {
            return Err(SetupCommandError::operational(
                "setup_notification_permission_check_failed",
                error,
            ))
        }
    };
    if permission == PermissionState::Granted {
        Ok(())
    } else {
        Err(SetupCommandError::new(
            "setup_notification_permission_denied",
        ))
    }
}

fn deliver_setup_completion(
    app: &tauri::AppHandle,
    copy: &SetupCopy,
) -> Result<(), SetupCommandError> {
    request_setup_notification_permission(app)?;
    app.notification()
        .builder()
        .title(&copy.notification_title)
        .body(&copy.notification_body)
        .group("oomu-first-run-setup")
        .auto_cancel()
        .show()
        .map_err(|error| {
            SetupCommandError::operational("setup_notification_delivery_failed", error)
        })
}

fn mark_setup_sample_failed(engine: &PersistenceEngine, sample_id: &str, code: &str) {
    if let Ok(connection) = engine.open_connection() {
        let _ = connection.execute(
            "UPDATE setup_sample_tasks SET state='failed',error_code=?2,updated_at_ms=?3 WHERE sample_id=?1",
            params![sample_id, code, unix_time_ms_i64()],
        );
    }
}

fn require_setup_durable_store(engine: &PersistenceEngine) -> Result<(), SetupCommandError> {
    engine
        .require_durable_store("run the first-use sample")
        .map_err(|error| SetupCommandError::operational("setup_storage_recovery_required", error))
}

fn prepare_setup_project(
    engine: &PersistenceEngine,
    route: &str,
    copy: &SetupCopy,
) -> Result<String, SetupCommandError> {
    let desired_policy = if route == "local" {
        "local_only"
    } else {
        "allow_configured_cloud"
    };
    let now = unix_time_ms_i64();
    let stale_before = now - 5 * 60 * 1_000;
    let connection = engine
        .open_connection()
        .map_err(SetupCommandError::internal)?;
    let fresh_running: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM setup_sample_tasks WHERE state='running' AND updated_at_ms>=?1",
            params![stale_before],
            |row| row.get(0),
        )
        .map_err(SetupCommandError::internal)?;
    if fresh_running > 0 {
        return Err(SetupCommandError::new("setup_sample_already_running"));
    }
    connection
        .execute(
            "UPDATE setup_sample_tasks SET state='failed',error_code='interrupted_retry',updated_at_ms=?2 WHERE state='running' AND updated_at_ms<?1",
            params![stale_before, now],
        )
        .map_err(SetupCommandError::internal)?;
    let reusable = connection
        .query_row(
            "SELECT sample.project_id FROM setup_sample_tasks sample JOIN projects project ON project.project_id=sample.project_id JOIN project_policy policy ON policy.project_id=project.project_id WHERE sample.state='failed' AND project.archived_at_ms IS NULL AND policy.data_policy=?1 ORDER BY sample.updated_at_ms DESC LIMIT 1",
            params![desired_policy],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(SetupCommandError::internal)?;
    drop(connection);
    if let Some(project_id) = reusable {
        return Ok(project_id);
    }
    crate::projects::repository::create(
        engine,
        CreateProjectRequest {
            name: copy.project_name.clone(),
            description: copy.project_description.clone(),
            data_policy: if route == "local" {
                ProjectDataPolicy::LocalOnly
            } else {
                ProjectDataPolicy::AllowConfiguredCloud
            },
        },
    )
    .map(|project| project.project_id)
    .map_err(SetupCommandError::internal)
}

fn prepare_setup_session(
    engine: &PersistenceEngine,
    project_id: &str,
    route: &str,
    copy: &SetupCopy,
) -> Result<String, SetupCommandError> {
    let existing = engine
        .open_connection()
        .map_err(SetupCommandError::internal)?
        .query_row(
            "SELECT id FROM chat_sessions WHERE project_id=?1 AND agent_id='setup-guide' AND model_id=?2 ORDER BY updated_at_ms DESC LIMIT 1",
            params![project_id, route],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(SetupCommandError::internal)?;
    if let Some(session_id) = existing {
        return Ok(session_id);
    }
    let session = engine
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "setup-guide".to_string(),
            provider_id: if route == "local" {
                "local_model".to_string()
            } else {
                "cloud_provider".to_string()
            },
            model_id: route.to_string(),
            title: Some(copy.project_name.clone()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .map_err(SetupCommandError::internal)?;
    crate::projects::repository::bind_record(
        engine,
        crate::projects::BindProjectRecordRequest {
            project_id: Some(project_id.to_string()),
            record_kind: "chat_session".to_string(),
            record_id: session.id.clone(),
        },
    )
    .map_err(SetupCommandError::internal)?;
    Ok(session.id)
}

fn persist_setup_sample_activation(
    engine: &PersistenceEngine,
    project_id: &str,
    task_run_id: &str,
    route: &str,
    delivery: &str,
    complete_setup: bool,
) -> Result<SetupState, SetupCommandError> {
    let connection = engine
        .open_connection()
        .map_err(SetupCommandError::internal)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(SetupCommandError::internal)?;
    let verified_at = unix_time_ms_i64();
    transaction.execute("INSERT INTO activation_receipts (receipt_id,project_id,task_run_id,model_route,capability_snapshot_json,verified_at_ms) VALUES (?1,?2,?3,?4,?5,?6)", params![format!("receipt_{}",crate::p0_contracts::TaskId::new()),project_id,task_run_id,route,json!({"model":"verified","task":"completed","delivery":delivery}).to_string(),verified_at]).map_err(SetupCommandError::internal)?;
    if complete_setup {
        transaction.execute("UPDATE setup_progress SET current_step='finished',completion_channel='local',sample_project_id=?1,completed_at_ms=?2,updated_at_ms=?2 WHERE singleton=1", params![project_id,verified_at]).map_err(SetupCommandError::internal)?;
    } else {
        transaction
            .execute(
                "UPDATE setup_progress SET sample_project_id=?1,updated_at_ms=?2 WHERE singleton=1",
                params![project_id, verified_at],
            )
            .map_err(SetupCommandError::internal)?;
    }
    transaction.commit().map_err(SetupCommandError::internal)?;
    repository::setup_state(engine).map_err(SetupCommandError::internal)
}

#[tauri::command]
pub async fn get_setup_state(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SetupState, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::setup_state(&engine)).await
}

#[tauri::command]
pub async fn save_setup_progress(
    request: SaveSetupProgressRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<SetupState, SetupCommandError> {
    if request.current_step.trim() == "sample"
        && request
            .completion_channel
            .as_deref()
            .is_none_or(|channel| channel == "local")
    {
        if let Err(error) = request_setup_notification_permission(&app) {
            eprintln!(
                "SETUP_NOTIFICATION_PERMISSION_UNAVAILABLE {}",
                crate::redaction::redacted_log_text(&error.code)
            );
        }
    }
    let engine = persistence.inner().clone();
    blocking(move || {
        repository::save_setup(
            &engine,
            request.current_step.trim(),
            request.model_path.as_deref(),
            request.completion_channel.as_deref(),
        )
    })
    .await
    .map_err(SetupCommandError::internal)
}

#[tauri::command]
pub async fn run_setup_sample_task(
    request: RunSetupSampleRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    gemma: tauri::State<'_, GemmaService>,
    manager: tauri::State<'_, AgentManager>,
    app: tauri::AppHandle,
) -> Result<SetupState, SetupCommandError> {
    let engine = persistence.inner().clone();
    // The sample creates a real Project and a durable activation receipt. A
    // recovery/volatile store cannot honestly complete either operation. Fail
    // at the boundary with a repairable, typed result instead of allowing the
    // lower-level Project guard to collapse into a generic internal error.
    require_setup_durable_store(&engine)?;
    let route = request.model_route.trim().to_string();
    let copy = SetupCopy::load(&engine)?;
    let project_id = prepare_setup_project(&engine, &route, &copy)?;
    let sample_id = format!("setup_{}", crate::p0_contracts::TaskId::new());
    let now = unix_time_ms_i64();
    engine.open_connection().map_err(SetupCommandError::internal)?.execute("INSERT INTO setup_sample_tasks (sample_id,project_id,state,created_at_ms,updated_at_ms) VALUES (?1,?2,'running',?3,?3)", params![sample_id,project_id,now]).map_err(SetupCommandError::internal)?;
    let execution: Result<(String, String), SetupCommandError> = async {
        let session_id = prepare_setup_session(&engine, &project_id, &route, &copy)?;
        let output = if route == "local" {
            let service = gemma.inner().clone();
            let local_sample_id = sample_id.clone();
            let local_copy = copy.clone();
            tauri::async_runtime::spawn_blocking(move || {
                run_local_setup_sample(&service, &local_sample_id, &local_copy)
            })
            .await
            .map_err(SetupCommandError::internal)??
        } else {
            let config = manager
                .select_provider_config(&route)
                .map_err(|error| SetupCommandError::operational("setup_provider_not_found", error))?
                .ok_or_else(|| SetupCommandError::new("setup_provider_not_found"))?;
            if config.auth_method != "api_key" || !config.credential_configured {
                return Err(SetupCommandError::new("setup_provider_credentials_missing"));
            }
            crate::projects::evaluate_project_policy(
                &engine,
                crate::projects::ProjectTransmissionRequest {
                    project_id: project_id.clone(),
                    task_id: None,
                    destination_kind: "provider".to_string(),
                    destination_origin: config.base_url.clone(),
                    data_classes: vec!["setup_sample_prompt".to_string()],
                    consent: true,
                },
            )
            .map_err(|error| {
                SetupCommandError::operational("setup_project_policy_denied", error)
            })?;
            let model_id = config
                .custom_model_ids
                .split([',', '\n'])
                .map(str::trim)
                .find(|value| !value.is_empty())
                .ok_or_else(|| SetupCommandError::new("setup_provider_model_missing"))?
                .to_string();
            inference::run_provider_inference(InferenceRequest {
                provider_id: config.provider_id,
                model_id,
                system_prompt: Some(copy.model_system_prompt.clone()),
                messages: vec![InferenceMessage {
                    role: "user".to_string(),
                    content: copy.model_prompt.clone(),
                    attachments: vec![],
                }],
                prompt: None,
                temperature: Some(0.0),
                max_tokens: Some(80),
                reasoning: None,
                reasoning_budget_tokens: None,
                base_url: Some(config.base_url),
                api_key_label: Some(config.api_key_label),
                api_key: config.api_key,
            })
            .await
            .map(|response| response.text.trim().to_string())
            .map_err(|error| {
                SetupCommandError::operational("setup_model_execution_failed", error.message)
            })?
        };
        Ok((session_id, output))
    }
    .await;
    let (session_id, output) = match execution {
        Ok((_, output)) if output.trim().is_empty() => {
            mark_setup_sample_failed(&engine, &sample_id, "model_output_empty");
            return Err(SetupCommandError::new("setup_model_output_empty"));
        }
        Ok(execution) => execution,
        Err(error) => {
            mark_setup_sample_failed(&engine, &sample_id, &error.code);
            return Err(error);
        }
    };
    let finalization = (|| -> Result<SetupState, SetupCommandError> {
        let connection = engine
            .open_connection()
            .map_err(SetupCommandError::internal)?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(SetupCommandError::internal)?;
        transaction.execute("UPDATE setup_sample_tasks SET state='completed',output_digest=?2,error_code=NULL,updated_at_ms=?3 WHERE sample_id=?1", params![sample_id,sha256_hex(output.as_bytes()),unix_time_ms_i64()]).map_err(SetupCommandError::internal)?;
        transaction.execute("INSERT INTO taskflows (flow_id,mission_id,parent_session_id,directive,status,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,'verified',?5,?5)",params![sample_id,format!("mission-{sample_id}"),session_id,&copy.task_directive,unix_time_ms_i64()]).map_err(SetupCommandError::internal)?;
        transaction.commit().map_err(SetupCommandError::internal)?;
        crate::tasks::reconcile_all(&engine).map_err(SetupCommandError::internal)?;
        let task_run_id: String = connection.query_row("SELECT task_run_id FROM task_runs WHERE runtime_kind='taskflow' AND runtime_record_id=?1", params![sample_id], |row| row.get(0)).map_err(SetupCommandError::internal)?;
        let delivery = match deliver_setup_completion(&app, &copy) {
            Ok(()) => "delivered",
            Err(error) => {
                eprintln!(
                    "SETUP_COMPLETION_NOTIFICATION_FAILED {}",
                    crate::redaction::redacted_log_text(&error.code)
                );
                "unavailable"
            }
        };
        persist_setup_sample_activation(
            &engine,
            &project_id,
            &task_run_id,
            &route,
            delivery,
            request.complete_setup,
        )
    })();
    match finalization {
        Ok(state) => Ok(state),
        Err(error) => {
            mark_setup_sample_failed(&engine, &sample_id, "sample_finalization_failed");
            Err(SetupCommandError::operational(
                "setup_finalization_failed",
                error.code,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine(label: &str) -> (std::path::PathBuf, PersistenceEngine) {
        let root = std::env::temp_dir().join(format!(
            "oomu-{label}-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        (root, engine)
    }

    fn activation_fixture(engine: &PersistenceEngine) -> (String, String) {
        let project = crate::projects::repository::create(
            engine,
            CreateProjectRequest {
                name: "Setup activation fixture".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = crate::p0_contracts::TaskId::new().to_string();
        let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
        let now = unix_time_ms_i64();
        engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,completed_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow',?4,'completed','test',?2,'Verified setup',?5,?5,?5,'reconciled')", params![task_run_id,task_id,project.project_id,format!("setup-{task_run_id}"),now]).unwrap();
        (project.project_id, task_run_id)
    }

    #[test]
    fn setup_sample_request_defaults_to_completion_and_accepts_deferred_mode() {
        let legacy: RunSetupSampleRequest =
            serde_json::from_value(json!({ "modelRoute": "local" })).unwrap();
        let deferred: RunSetupSampleRequest =
            serde_json::from_value(json!({ "modelRoute": "local", "completeSetup": false }))
                .unwrap();

        assert!(legacy.complete_setup);
        assert!(!deferred.complete_setup);
    }

    #[test]
    fn deferred_sample_persists_receipt_and_keeps_setup_usable() {
        let (root, engine) = test_engine("setup-deferred-activation");
        repository::save_setup(&engine, "permissions", Some("local"), Some("local")).unwrap();
        let (project_id, task_run_id) = activation_fixture(&engine);

        let state = persist_setup_sample_activation(
            &engine,
            &project_id,
            &task_run_id,
            "local",
            "delivered",
            false,
        )
        .unwrap();

        assert_eq!(state.current_step, "permissions");
        assert_eq!(
            state.sample_project_id.as_deref(),
            Some(project_id.as_str())
        );
        assert_eq!(state.completed_at_ms, None);
        assert_eq!(
            repository::setup_state(&engine).unwrap().current_step,
            "permissions"
        );
        assert_eq!(
            repository::save_setup(&engine, "connectors", None, Some("local"))
                .unwrap()
                .current_step,
            "connectors"
        );
        let receipt_count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM activation_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(receipt_count, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_sample_preserves_the_existing_finish_behavior() {
        let (root, engine) = test_engine("setup-completed-activation");
        repository::save_setup(&engine, "sample", Some("local"), Some("telegram")).unwrap();
        let (project_id, task_run_id) = activation_fixture(&engine);

        let state = persist_setup_sample_activation(
            &engine,
            &project_id,
            &task_run_id,
            "local",
            "delivered",
            true,
        )
        .unwrap();

        assert_eq!(state.current_step, "finished");
        assert_eq!(state.completion_channel.as_deref(), Some("local"));
        assert_eq!(
            state.sample_project_id.as_deref(),
            Some(project_id.as_str())
        );
        assert!(state.completed_at_ms.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn setup_step_names_fail_closed() {
        let (root, engine) = test_engine("setup-steps");
        assert!(repository::save_setup(&engine, "invented", None, None).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn local_setup_request_uses_the_visible_chat_contract() {
        let (root, engine) = test_engine("setup-localized-prompt");
        let copy = SetupCopy::load(&engine).unwrap();
        let request = setup_local_infer_request("setup-contract", 2, &copy);
        assert!(request.prompt_is_full_context);
        assert!(request.deterministic);
        assert_eq!(request.max_tokens, Some(80));
        assert_eq!(
            request.session_id.as_deref(),
            Some("setup-contract-attempt-2")
        );
        assert!(request
            .prompt
            .ends_with("<|turn>model\n<|channel>text\n<channel|>"));
        assert!(request.prompt.contains(&copy.model_prompt));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn selecting_local_delivery_replaces_a_saved_remote_value() {
        let (root, engine) = test_engine("setup-delivery");
        repository::save_setup(&engine, "connectors", Some("local"), Some("telegram")).unwrap();
        let state =
            repository::save_setup(&engine, "sample", Some("local"), Some("local")).unwrap();
        assert_eq!(state.completion_channel.as_deref(), Some("local"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn volatile_storage_blocks_the_sample_before_project_creation() {
        let root = std::env::temp_dir().join(format!(
            "oomu-setup-volatile-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_volatile_at(root.join("state.sqlite")).unwrap();

        let error = require_setup_durable_store(&engine).unwrap_err();

        assert_eq!(error.code, "setup_storage_recovery_required");
        let project_count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(project_count, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_setup_sample_never_remains_running() {
        let (root, engine) = test_engine("setup-failure");
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "Setup failure fixture".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let sample_id = "setup_failure_fixture";
        let now = unix_time_ms_i64();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO setup_sample_tasks (sample_id,project_id,state,created_at_ms,updated_at_ms) VALUES (?1,?2,'running',?3,?3)",
                params![sample_id, project.project_id, now],
            )
            .unwrap();
        mark_setup_sample_failed(&engine, sample_id, "model_output_empty");
        let (state, code): (String, Option<String>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state,error_code FROM setup_sample_tasks WHERE sample_id=?1",
                params![sample_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "failed");
        assert_eq!(code.as_deref(), Some("model_output_empty"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_setup_retry_reuses_its_project() {
        let (root, engine) = test_engine("setup-retry");
        let copy = SetupCopy::load(&engine).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "First OOMU Project".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let old = unix_time_ms_i64() - 10 * 60 * 1_000;
        engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO setup_sample_tasks (sample_id,project_id,state,created_at_ms,updated_at_ms) VALUES ('setup_interrupted',?1,'running',?2,?2)",
                params![project.project_id, old],
            )
            .unwrap();

        let retried_project = prepare_setup_project(&engine, "local", &copy).unwrap();
        assert_eq!(retried_project, project.project_id);
        let (state, code): (String, Option<String>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state,error_code FROM setup_sample_tasks WHERE sample_id='setup_interrupted'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "failed");
        assert_eq!(code.as_deref(), Some("interrupted_retry"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn setup_retry_reuses_its_bound_chat_session() {
        let (root, engine) = test_engine("setup-session-retry");
        let copy = SetupCopy::load(&engine).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "First OOMU Project".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let first = prepare_setup_session(&engine, &project.project_id, "local", &copy).unwrap();
        let second = prepare_setup_session(&engine, &project.project_id, "local", &copy).unwrap();
        assert_eq!(first, second);
        let count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM chat_sessions WHERE project_id=?1 AND agent_id='setup-guide'",
                params![project.project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
