use crate::foundation::{clock::unix_time_ms_u64, digest::sha256_hex};
use crate::sovereign_identity::SovereignIdentity;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::Manager;

pub(crate) mod shield_decision;

const MAX_PROOF_TTL_MS: u64 = 15 * 60 * 1_000;
const MAX_AUTHORIZED_STEPS: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAuthorityError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct NativeAuthorityProof {
    pub proof_id: String,
    pub actor_id: String,
    pub session_id: String,
    pub operation_classes: Vec<String>,
    pub canonical_scopes: Vec<String>,
    pub max_steps: usize,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
    pub persistence: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestNativeAuthorityProof {
    pub session_id: String,
    pub operation_classes: Vec<String>,
    pub scopes: Vec<String>,
    pub max_steps: usize,
    pub persistence: String,
    #[serde(default)]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAuthorityProofResponse {
    pub proof_id: String,
    pub expires_at_ms: u64,
    pub persistence: String,
}

#[derive(Debug, Clone)]
pub struct NativeAuthorityExpectation {
    pub actor_id: String,
    pub session_id: String,
    pub operation_classes: Vec<String>,
    pub canonical_scopes: Vec<String>,
    pub max_steps: usize,
    pub allowed_persistences: Vec<String>,
}

#[derive(Clone, Default)]
pub struct NativeAuthorityManager {
    proofs: Arc<Mutex<HashMap<String, NativeAuthorityProof>>>,
}

impl NativeAuthorityManager {
    fn insert_after_presence(
        &self,
        actor_id: String,
        request: RequestNativeAuthorityProof,
    ) -> Result<NativeAuthorityProofResponse, NativeAuthorityError> {
        let session_id = required_value("session_id", &request.session_id)?;
        let operation_classes = canonical_values("operation_classes", request.operation_classes)?;
        let canonical_scopes = canonical_scopes(request.scopes)?;
        validate_steps(request.max_steps)?;
        let persistence = canonical_persistence(&request.persistence)?;
        let issued_at_ms = unix_time_ms_u64();
        let expires_at_ms = issued_at_ms.saturating_add(MAX_PROOF_TTL_MS);
        let nonce = random_token();
        let proof_id = format!(
            "authority_{}",
            sha256_hex(format!("{actor_id}:{session_id}:{nonce}:{issued_at_ms}").as_bytes())
        );
        let proof = NativeAuthorityProof {
            proof_id: proof_id.clone(),
            actor_id,
            session_id,
            operation_classes,
            canonical_scopes,
            max_steps: request.max_steps,
            issued_at_ms,
            expires_at_ms,
            nonce,
            persistence,
        };
        let selected_persistence = proof.persistence.clone();
        self.proofs
            .lock()
            .map_err(|_| {
                authority_error(
                    "authority_proof_store_unavailable",
                    "Unable to secure this approval.",
                )
            })?
            .insert(proof_id.clone(), proof);
        Ok(NativeAuthorityProofResponse {
            proof_id,
            expires_at_ms,
            persistence: selected_persistence,
        })
    }

    pub fn consume(
        &self,
        proof_id: &str,
        mut expected: NativeAuthorityExpectation,
    ) -> Result<NativeAuthorityProof, NativeAuthorityError> {
        expected.operation_classes =
            canonical_values("operation_classes", expected.operation_classes)?;
        expected.canonical_scopes = canonical_scopes(expected.canonical_scopes)?;
        validate_steps(expected.max_steps)?;
        expected.allowed_persistences = expected
            .allowed_persistences
            .iter()
            .map(|value| canonical_persistence(value))
            .collect::<Result<Vec<_>, _>>()?;
        let proof_id = required_value("proof_id", proof_id)?;
        let mut proofs = self.proofs.lock().map_err(|_| {
            authority_error(
                "authority_proof_store_unavailable",
                "Unable to verify this approval.",
            )
        })?;
        let proof = proofs.remove(&proof_id).ok_or_else(|| {
            authority_error(
                "authority_proof_missing",
                "This approval is no longer available.",
            )
        })?;
        let now = unix_time_ms_u64();
        if now > proof.expires_at_ms
            || proof.expires_at_ms.saturating_sub(proof.issued_at_ms) > MAX_PROOF_TTL_MS
        {
            return Err(authority_error(
                "authority_proof_expired",
                "This approval has expired.",
            ));
        }
        if proof.actor_id != expected.actor_id {
            return Err(authority_error(
                "authority_actor_mismatch",
                "This approval belongs to a different local identity.",
            ));
        }
        if proof.session_id != expected.session_id {
            return Err(authority_error(
                "authority_session_mismatch",
                "This approval belongs to a different session.",
            ));
        }
        if proof.operation_classes != expected.operation_classes {
            return Err(authority_error(
                "authority_operation_mismatch",
                "This approval does not cover that action.",
            ));
        }
        if proof.canonical_scopes != expected.canonical_scopes {
            return Err(authority_error(
                "authority_scope_mismatch",
                "This approval does not cover that location.",
            ));
        }
        if proof.max_steps != expected.max_steps {
            return Err(authority_error(
                "authority_step_mismatch",
                "This approval has a different action limit.",
            ));
        }
        if !expected
            .allowed_persistences
            .iter()
            .any(|value| value == &proof.persistence)
        {
            return Err(authority_error(
                "authority_persistence_mismatch",
                "This approval has a different duration.",
            ));
        }
        Ok(proof)
    }

    pub async fn request_after_native_presence(
        &self,
        app: &tauri::AppHandle,
        actor_id: String,
        request: RequestNativeAuthorityProof,
    ) -> Result<NativeAuthorityProofResponse, NativeAuthorityError> {
        let session_id = required_value("session_id", &request.session_id)?;
        let operation_classes =
            canonical_values("operation_classes", request.operation_classes.clone())?;
        let scopes = canonical_scopes(request.scopes.clone())?;
        validate_steps(request.max_steps)?;
        let persistence = canonical_persistence(&request.persistence)?;
        let copy = native_prompt_copy(
            request.locale.as_deref().unwrap_or("en-US"),
            &operation_classes,
            &scopes,
            &persistence,
            request.max_steps,
        );
        #[cfg(debug_assertions)]
        let automated_persistence =
            crate::scenario_one_e2e_profile::automated_native_authority_persistence(
                &crate::scenario_one_e2e_profile::NativeAuthorityProbe {
                    session_id: &request.session_id,
                    operation_classes: &request.operation_classes,
                    scopes: &request.scopes,
                    max_steps: request.max_steps,
                    persistence: &request.persistence,
                    locale: request.locale.as_deref(),
                },
            );
        #[cfg(not(debug_assertions))]
        let automated_persistence: Option<String> = None;
        if let Some(automated_persistence) = automated_persistence {
            let selected_persistence = scenario_one_native_authority_prompt(
                app,
                copy,
                persistence.as_str(),
                &automated_persistence,
            )
            .await?;
            return self.insert_after_presence(
                actor_id,
                RequestNativeAuthorityProof {
                    session_id,
                    operation_classes,
                    scopes,
                    max_steps: request.max_steps,
                    persistence: selected_persistence,
                    locale: request.locale,
                },
            );
        }
        let NativePromptCopy {
            title,
            body,
            allow_once,
            allow_persistent,
            deny,
        } = copy;
        let main_window = app.get_webview_window("main").ok_or_else(|| {
            authority_error(
                "authority_native_prompt_window_unavailable",
                "OOMU could not attach the approval prompt to its window.",
            )
        })?;
        main_window.unminimize().map_err(|_| {
            authority_error(
                "authority_native_prompt_window_unavailable",
                "OOMU could not restore its window for the approval prompt.",
            )
        })?;
        main_window.show().map_err(|_| {
            authority_error(
                "authority_native_prompt_window_unavailable",
                "OOMU could not show its window for the approval prompt.",
            )
        })?;
        main_window.set_focus().map_err(|_| {
            authority_error(
                "authority_native_prompt_window_unavailable",
                "OOMU could not focus its window for the approval prompt.",
            )
        })?;
        let dialog = rfd::AsyncMessageDialog::new()
            .set_parent(&main_window)
            .set_title(title)
            .set_description(body)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                allow_once.clone(),
                allow_persistent.clone(),
                deny,
            ));
        let approved = tokio::time::timeout(Duration::from_secs(180), dialog.show())
            .await
            .map_err(|_| {
                authority_error(
                    "authority_native_prompt_timeout",
                    "The approval prompt timed out. Return to OOMU and try again.",
                )
            })?;
        let selected_persistence = match approved {
            rfd::MessageDialogResult::Custom(value) if value == allow_once => "one_time",
            rfd::MessageDialogResult::Custom(value) if value == allow_persistent => {
                persistence.as_str()
            }
            _ => {
                return Err(authority_error(
                    "authority_user_denied",
                    "You chose not to allow this action.",
                ))
            }
        };
        self.insert_after_presence(
            actor_id,
            RequestNativeAuthorityProof {
                session_id,
                operation_classes,
                scopes,
                max_steps: request.max_steps,
                persistence: selected_persistence.to_string(),
                locale: request.locale,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn issue_test_harness(
        &self,
        actor_id: String,
        request: RequestNativeAuthorityProof,
    ) -> Result<NativeAuthorityProofResponse, NativeAuthorityError> {
        self.insert_after_presence(actor_id, request)
    }
}

#[tauri::command]
pub async fn request_native_authority(
    request: RequestNativeAuthorityProof,
    app: tauri::AppHandle,
    identity: tauri::State<'_, SovereignIdentity>,
    authority: tauri::State<'_, NativeAuthorityManager>,
) -> Result<NativeAuthorityProofResponse, NativeAuthorityError> {
    authority
        .request_after_native_presence(&app, current_actor_id(identity.inner())?, request)
        .await
}

#[derive(Clone)]
struct NativePromptCopy {
    title: String,
    body: String,
    allow_once: String,
    allow_persistent: String,
    deny: String,
}

#[cfg(all(debug_assertions, target_os = "macos"))]
async fn scenario_one_native_authority_prompt(
    app: &tauri::AppHandle,
    copy: NativePromptCopy,
    requested_persistence: &str,
    automated_persistence: &str,
) -> Result<String, NativeAuthorityError> {
    use objc2::{rc::autoreleasepool, sel, MainThreadMarker};
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSModalPanelRunLoopMode};
    use objc2_foundation::{NSArray, NSObjectNSDelayedPerforming, NSString};
    use tokio::sync::oneshot;

    let requested_persistence = requested_persistence.to_string();
    let automated_persistence = automated_persistence.to_string();
    let (sender, receiver) = oneshot::channel();
    app.run_on_main_thread(move || {
        autoreleasepool(|_| {
            let Some(mtm) = MainThreadMarker::new() else {
                let _ = sender.send(None);
                return;
            };
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(&NSString::from_str(&copy.title));
            alert.setInformativeText(&NSString::from_str(&copy.body));
            let allow_once = alert.addButtonWithTitle(&NSString::from_str(&copy.allow_once));
            let allow_persistent =
                alert.addButtonWithTitle(&NSString::from_str(&copy.allow_persistent));
            let deny = alert.addButtonWithTitle(&NSString::from_str(&copy.deny));
            deny.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            let automated_button = if automated_persistence == "one_time" {
                Some(allow_once)
            } else if automated_persistence == requested_persistence {
                Some(allow_persistent)
            } else {
                None
            };
            if let Some(button) = automated_button {
                // SAFETY: this activates the retained button on the real
                // AppKit alert while its native modal run loop is active.
                unsafe {
                    let modes = NSArray::from_slice(&[NSModalPanelRunLoopMode]);
                    button.performSelector_withObject_afterDelay_inModes(
                        sel!(performClick:),
                        None,
                        0.35,
                        &modes,
                    );
                }
                eprintln!(
                    "OOMU_SCENARIO_ONE_E2E_TRACE stage=plan_authority status=scheduled_real_button persistence={automated_persistence}"
                );
            }
            let response = alert.runModal();
            let selected = response
                .checked_sub(NSAlertFirstButtonReturn)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| match index {
                    0 => Some("one_time".to_string()),
                    1 => Some(requested_persistence),
                    _ => None,
                });
            let _ = sender.send(selected);
        });
    })
    .map_err(|_| {
        authority_error(
            "authority_native_prompt_failed",
            "OOMU could not present the native approval prompt.",
        )
    })?;
    receiver
        .await
        .map_err(|_| {
            authority_error(
                "authority_native_prompt_closed",
                "The native approval prompt closed without a decision.",
            )
        })?
        .ok_or_else(|| {
            authority_error(
                "authority_user_denied",
                "You chose not to allow this action.",
            )
        })
}

#[cfg(any(not(debug_assertions), not(target_os = "macos")))]
async fn scenario_one_native_authority_prompt(
    _app: &tauri::AppHandle,
    _copy: NativePromptCopy,
    _requested_persistence: &str,
    _automated_persistence: &str,
) -> Result<String, NativeAuthorityError> {
    Err(authority_error(
        "authority_native_prompt_unavailable",
        "Native Scenario approval is unavailable on this platform.",
    ))
}

fn native_prompt_copy(
    locale: &str,
    operations: &[String],
    scopes: &[String],
    persistence: &str,
    _max_steps: usize,
) -> NativePromptCopy {
    let mut operation_labels = operations
        .iter()
        .map(|operation| plain_operation(locale, operation))
        .collect::<Vec<_>>();
    operation_labels.sort();
    operation_labels.dedup();
    let operation = if operation_labels.len() > 3 {
        plain_operation_group(locale).to_string()
    } else {
        operation_labels.join(", ")
    };
    let place = scopes
        .iter()
        .map(|scope| plain_scope(locale, scope))
        .collect::<Vec<_>>()
        .join(", ");
    let (title, sentence, choice, allow_once, allow_session, allow_always, deny) = match locale {
        value if value.starts_with("de") => (
            "Diese Aktion erlauben?",
            format!("OOMU möchte {operation} in {place}."),
            "Sie entscheiden, wie lange.",
            "Einmal erlauben",
            "Für diese Sitzung",
            "Immer erlauben",
            "Nicht erlauben",
        ),
        value if value.starts_with("es") => (
            "¿Permitir esta acción?",
            format!("OOMU quiere {operation} en {place}."),
            "Tú decides durante cuánto tiempo.",
            "Permitir una vez",
            "Durante esta sesión",
            "Permitir siempre",
            "No permitir",
        ),
        value if value.starts_with("fr") => (
            "Autoriser cette action ?",
            format!("OOMU souhaite {operation} dans {place}."),
            "Vous choisissez la durée.",
            "Autoriser une fois",
            "Pour cette session",
            "Toujours autoriser",
            "Ne pas autoriser",
        ),
        value if value.starts_with("id") => (
            "Izinkan tindakan ini?",
            format!("OOMU ingin {operation} di {place}."),
            "Anda menentukan berapa lama.",
            "Izinkan sekali",
            "Untuk sesi ini",
            "Selalu izinkan",
            "Jangan izinkan",
        ),
        value if value.starts_with("ja") => (
            "この操作を許可しますか？",
            format!("OOMU が {place} で {operation}。"),
            "許可する期間を選べます。",
            "1回だけ許可",
            "このセッション中",
            "常に許可",
            "許可しない",
        ),
        value if value.starts_with("pt") => (
            "Permitir esta ação?",
            format!("O OOMU quer {operation} em {place}."),
            "Você escolhe por quanto tempo.",
            "Permitir uma vez",
            "Nesta sessão",
            "Permitir sempre",
            "Não permitir",
        ),
        value if value.starts_with("ru") => (
            "Разрешить это действие?",
            format!("OOMU хочет {operation} в {place}."),
            "Вы выбираете срок разрешения.",
            "Разрешить один раз",
            "Для этого сеанса",
            "Разрешать всегда",
            "Не разрешать",
        ),
        value if value.starts_with("uk") => (
            "Дозволити цю дію?",
            format!("OOMU хоче {operation} у {place}."),
            "Ви обираєте тривалість дозволу.",
            "Дозволити один раз",
            "Для цього сеансу",
            "Завжди дозволяти",
            "Не дозволяти",
        ),
        value if value.starts_with("vi") => (
            "Cho phép thao tác này?",
            format!("OOMU muốn {operation} trong {place}."),
            "Bạn chọn thời hạn cho phép.",
            "Chỉ cho phép lần này",
            "Trong phiên này",
            "Luôn cho phép",
            "Không cho phép",
        ),
        value if value.starts_with("zh") => (
            "允许此操作？",
            format!("OOMU 想要在{place}{operation}。"),
            "你可以选择允许时长。",
            "仅允许一次",
            "本次会话",
            "始终允许",
            "不允许",
        ),
        _ => (
            "Allow this action?",
            format!("OOMU wants to {operation} in {place}."),
            "You choose how long.",
            "Allow once",
            "For this session",
            "Always allow",
            "Don't allow",
        ),
    };
    NativePromptCopy {
        title: title.to_string(),
        body: format!("{sentence}\n\n{choice}"),
        allow_once: allow_once.to_string(),
        allow_persistent: if persistence == "global_trust" {
            allow_always
        } else {
            allow_session
        }
        .to_string(),
        deny: deny.to_string(),
    }
}

fn plain_operation(locale: &str, operation: &str) -> String {
    let label = match operation {
        "filesystem_read" => "read files",
        "filesystem_write" | "external_writes" => "change files",
        "shell_command" | "shell_commands" => "run a system command",
        "delete_file" => "delete files",
        "registered_task_tool" => "run the approved task",
        "codebase_patch" => "change project code",
        "codebase_compile" => "check the project build",
        _ => "perform the approved action",
    };
    match locale {
        value if value.starts_with("de") => match operation {
            "filesystem_read" => "Dateien lesen",
            "filesystem_write" | "external_writes" => "Dateien ändern",
            "delete_file" => "Dateien löschen",
            _ => "die genehmigte Aktion ausführen",
        },
        value if value.starts_with("es") => match operation {
            "filesystem_read" => "leer archivos",
            "filesystem_write" | "external_writes" => "cambiar archivos",
            "delete_file" => "eliminar archivos",
            _ => "realizar la acción aprobada",
        },
        value if value.starts_with("fr") => match operation {
            "filesystem_read" => "lire des fichiers",
            "filesystem_write" | "external_writes" => "modifier des fichiers",
            "delete_file" => "supprimer des fichiers",
            _ => "effectuer l’action approuvée",
        },
        value if value.starts_with("id") => match operation {
            "filesystem_read" => "membaca file",
            "filesystem_write" | "external_writes" => "mengubah file",
            "delete_file" => "menghapus file",
            _ => "melakukan tindakan yang disetujui",
        },
        value if value.starts_with("ja") => match operation {
            "filesystem_read" => "ファイルを読む",
            "filesystem_write" | "external_writes" => "ファイルを変更します",
            "delete_file" => "ファイルを削除します",
            _ => "承認済みの操作を実行します",
        },
        value if value.starts_with("pt") => match operation {
            "filesystem_read" => "ler arquivos",
            "filesystem_write" | "external_writes" => "alterar arquivos",
            "delete_file" => "apagar arquivos",
            _ => "realizar a ação aprovada",
        },
        value if value.starts_with("ru") => match operation {
            "filesystem_read" => "читать файлы",
            "filesystem_write" | "external_writes" => "изменять файлы",
            "delete_file" => "удалять файлы",
            _ => "выполнить одобренное действие",
        },
        value if value.starts_with("uk") => match operation {
            "filesystem_read" => "читати файли",
            "filesystem_write" | "external_writes" => "змінювати файли",
            "delete_file" => "видаляти файли",
            _ => "виконати схвалену дію",
        },
        value if value.starts_with("vi") => match operation {
            "filesystem_read" => "đọc tệp",
            "filesystem_write" | "external_writes" => "thay đổi tệp",
            "delete_file" => "xóa tệp",
            _ => "thực hiện thao tác đã duyệt",
        },
        value if value.starts_with("zh") => match operation {
            "filesystem_read" => "读取文件",
            "filesystem_write" | "external_writes" => "更改文件",
            "delete_file" => "删除文件",
            _ => "执行已批准的操作",
        },
        _ => label,
    }
    .to_string()
}

fn plain_operation_group(locale: &str) -> &'static str {
    match locale {
        value if value.starts_with("de") => "genehmigte lokale Aufgaben ausführen",
        value if value.starts_with("es") => "ejecutar las tareas locales aprobadas",
        value if value.starts_with("fr") => "exécuter les tâches locales approuvées",
        value if value.starts_with("id") => "menjalankan tugas lokal yang disetujui",
        value if value.starts_with("ja") => "承認済みのローカルタスクを実行します",
        value if value.starts_with("pt") => "executar as tarefas locais aprovadas",
        value if value.starts_with("ru") => "выполнить одобренные локальные задачи",
        value if value.starts_with("uk") => "виконати схвалені локальні завдання",
        value if value.starts_with("vi") => "chạy các tác vụ cục bộ đã duyệt",
        value if value.starts_with("zh") => "执行已批准的本地任务",
        _ => "run the approved local tasks",
    }
}

fn plain_scope(locale: &str, scope: &str) -> String {
    if !scope.starts_with("actuation-session:") {
        return scope.to_string();
    }
    match locale {
        value if value.starts_with("de") => "dieser Aufgabe",
        value if value.starts_with("es") => "esta tarea",
        value if value.starts_with("fr") => "cette tâche",
        value if value.starts_with("id") => "tugas ini",
        value if value.starts_with("ja") => "このタスク",
        value if value.starts_with("pt") => "esta tarefa",
        value if value.starts_with("ru") => "этой задаче",
        value if value.starts_with("uk") => "цьому завданні",
        value if value.starts_with("vi") => "tác vụ này",
        value if value.starts_with("zh") => "此任务中",
        _ => "this task",
    }
    .to_string()
}

pub fn current_actor_id(identity: &SovereignIdentity) -> Result<String, NativeAuthorityError> {
    identity
        .profile()
        .map(|profile| profile.fingerprint)
        .map_err(|_| {
            authority_error(
                "authority_identity_unavailable",
                "OOMU could not verify your local identity.",
            )
        })
}

pub fn canonical_scope(scope: &str) -> Result<String, NativeAuthorityError> {
    let scope = required_value("scope", scope)?;
    if scope.starts_with("actuation-session:") {
        return Ok(scope);
    }
    let path = expand_home(&scope);
    canonicalize_allowing_missing_leaf(&path)
        .map(|value| value.to_string_lossy().to_string())
        .map_err(|_| {
            authority_error(
                "authority_scope_invalid",
                "OOMU could not verify that location.",
            )
        })
}

fn canonical_scopes(scopes: Vec<String>) -> Result<Vec<String>, NativeAuthorityError> {
    let mut values = scopes
        .iter()
        .map(|scope| canonical_scope(scope))
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    values.dedup();
    if values.is_empty() {
        return Err(authority_error(
            "authority_scope_missing",
            "Choose a location for this action.",
        ));
    }
    Ok(values)
}

fn canonical_values(
    label: &'static str,
    values: Vec<String>,
) -> Result<Vec<String>, NativeAuthorityError> {
    let mut values = values
        .into_iter()
        .map(|value| {
            required_value(label, &value).map(|value| value.to_ascii_lowercase().replace('-', "_"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    values.dedup();
    if values.is_empty() {
        return Err(authority_error(
            "authority_operation_missing",
            "Choose an action to allow.",
        ));
    }
    Ok(values)
}

fn canonical_persistence(value: &str) -> Result<String, NativeAuthorityError> {
    let value = required_value("persistence", value)?
        .to_ascii_lowercase()
        .replace('-', "_");
    match value.as_str() {
        "one_time" | "session_gated" | "global_trust" => Ok(value),
        _ => Err(authority_error(
            "authority_persistence_invalid",
            "That permission duration is not available.",
        )),
    }
}

fn validate_steps(max_steps: usize) -> Result<(), NativeAuthorityError> {
    if !(1..=MAX_AUTHORIZED_STEPS).contains(&max_steps) {
        return Err(authority_error(
            "authority_steps_invalid",
            "Choose between 1 and 50 actions.",
        ));
    }
    Ok(())
}

fn required_value(label: &'static str, value: &str) -> Result<String, NativeAuthorityError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(authority_error(
            "authority_value_missing",
            &format!("{label} is required."),
        ));
    }
    Ok(value.to_string())
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn canonicalize_allowing_missing_leaf(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let canonical_parent = fs::canonicalize(parent)?;
    Ok(path
        .file_name()
        .map(|name| canonical_parent.join(name))
        .unwrap_or(canonical_parent))
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn authority_error(code: &'static str, message: &str) -> NativeAuthorityError {
    NativeAuthorityError {
        code,
        boundary: "NativeAuthorityBoundary",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_request(session: &str) -> RequestNativeAuthorityProof {
        RequestNativeAuthorityProof {
            session_id: session.to_string(),
            operation_classes: vec!["external_writes".to_string()],
            scopes: vec![std::env::temp_dir().to_string_lossy().to_string()],
            max_steps: 1,
            persistence: "one_time".to_string(),
            locale: None,
        }
    }

    #[test]
    fn authority_boundary_proof_is_single_use_and_session_bound() {
        let manager = NativeAuthorityManager::default();
        let response = manager
            .issue_test_harness("actor-a".to_string(), fixture_request("session-a"))
            .unwrap();
        let scope = canonical_scope(&std::env::temp_dir().to_string_lossy()).unwrap();
        let expected = NativeAuthorityExpectation {
            actor_id: "actor-a".to_string(),
            session_id: "session-a".to_string(),
            operation_classes: vec!["external_writes".to_string()],
            canonical_scopes: vec![scope],
            max_steps: 1,
            allowed_persistences: vec!["one_time".to_string()],
        };
        assert!(manager
            .consume(&response.proof_id, expected.clone())
            .is_ok());
        assert_eq!(
            manager
                .consume(&response.proof_id, expected)
                .unwrap_err()
                .code,
            "authority_proof_missing"
        );
    }

    #[test]
    fn authority_boundary_wrong_session_consumes_and_denies_proof() {
        let manager = NativeAuthorityManager::default();
        let response = manager
            .issue_test_harness("actor-a".to_string(), fixture_request("session-a"))
            .unwrap();
        let expected = NativeAuthorityExpectation {
            actor_id: "actor-a".to_string(),
            session_id: "session-b".to_string(),
            operation_classes: vec!["external_writes".to_string()],
            canonical_scopes: vec![
                canonical_scope(&std::env::temp_dir().to_string_lossy()).unwrap()
            ],
            max_steps: 1,
            allowed_persistences: vec!["one_time".to_string()],
        };
        assert_eq!(
            manager
                .consume(&response.proof_id, expected)
                .unwrap_err()
                .code,
            "authority_session_mismatch"
        );
    }

    #[test]
    fn authority_boundary_renderer_requests_require_native_proof_fields() {
        let trust_request = serde_json::json!({
            "sessionId": "session-a",
            "directoryPath": std::env::temp_dir(),
            "allowedToolCategories": ["external_writes"],
            "permissionLevel": "one_time"
        });
        assert!(
            serde_json::from_value::<super::super::db::UpsertSovereignTrustPolicyRequest>(
                trust_request.clone()
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<super::super::db::ActivateSovereignTrustSessionRequest>(
                trust_request
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<super::super::shield_gate::GrantActuationLeaseRequest>(
                serde_json::json!({
                    "sessionId": "session-a",
                    "durationMs": 60_000,
                    "maxSteps": 1,
                    "operationClasses": ["filesystem_write"]
                })
            )
            .is_err()
        );
    }
}
