use crate::{
    foundation::{clock::unix_time_ms_u64, digest::sha256_hex},
    shield_gate::{ShieldApprovalDecision, ShieldApprovalRequest},
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use super::{authority_error, NativeAuthorityError};

const SHIELD_DECISION_SCHEMA_VERSION: u32 = 1;
const SHIELD_DECISION_TTL_MS: u64 = 2 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrozenShieldRequest {
    pub approval_token: String,
    pub request_sha256: String,
    pub origin_kind: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub generation_token: Option<String>,
    pub task_run_id: Option<String>,
    pub action_class: String,
    pub normalized_action_kind: String,
    pub argument_class: String,
    pub argument_sha256: String,
    pub canonical_resource: String,
    pub permitted_scope_kinds: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeShieldDecision {
    pub decision_id: String,
    pub approval_token: String,
    pub request_sha256: String,
    pub actor_id: String,
    pub origin_kind: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub generation_token: Option<String>,
    pub task_run_id: Option<String>,
    pub action_class: String,
    pub normalized_action_kind: String,
    pub argument_class: String,
    pub argument_sha256: String,
    pub canonical_resource: String,
    pub scope_kind: String,
    pub decision: ShieldApprovalDecision,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

#[derive(Clone, Default)]
pub(crate) struct NativeShieldDecisionStore {
    decisions: Arc<Mutex<HashMap<String, NativeShieldDecision>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeShieldPromptSelection {
    pub decision: ShieldApprovalDecision,
    pub scope_kind: String,
}

#[derive(Serialize)]
struct CanonicalShieldRequest<'a> {
    schema_version: u32,
    approval_token_sha256: String,
    origin_kind: &'a str,
    session_id: &'a Option<String>,
    turn_id: &'a Option<String>,
    generation_token: &'a Option<String>,
    task_run_id: &'a Option<String>,
    project_id: &'a Option<String>,
    principal: &'a Option<String>,
    normalized_action_kind: &'a str,
    action_class: &'a str,
    argument_class: &'a str,
    argument_sha256: &'a str,
    canonical_resource: &'a str,
    target_path: &'a Option<String>,
    mandatory_reconfirm: bool,
    permitted_scope_kinds: &'a [String],
}

impl NativeShieldDecisionStore {
    pub(crate) fn issue_after_native_presence(
        &self,
        frozen: &FrozenShieldRequest,
        actor_id: String,
        selection: NativeShieldPromptSelection,
    ) -> Result<String, NativeAuthorityError> {
        let actor_id = required("actor_id", &actor_id)?;
        if !frozen
            .permitted_scope_kinds
            .iter()
            .any(|kind| kind == &selection.scope_kind)
        {
            return Err(authority_error(
                "shield_decision_scope_invalid",
                "That permission duration is not available for this action.",
            ));
        }
        if selection.decision == ShieldApprovalDecision::Deny && selection.scope_kind != "once" {
            return Err(authority_error(
                "shield_decision_denial_scope_invalid",
                "A denied action cannot create a permission scope.",
            ));
        }
        let issued_at_ms = unix_time_ms_u64();
        let expires_at_ms = issued_at_ms.saturating_add(SHIELD_DECISION_TTL_MS);
        let nonce = random_token();
        let decision_id = format!(
            "shielddecision_{}",
            sha256_hex(
                format!(
                    "{}:{}:{}:{}",
                    frozen.request_sha256, actor_id, nonce, issued_at_ms
                )
                .as_bytes(),
            )
        );
        let decision = NativeShieldDecision {
            decision_id: decision_id.clone(),
            approval_token: frozen.approval_token.clone(),
            request_sha256: frozen.request_sha256.clone(),
            actor_id,
            origin_kind: frozen.origin_kind.clone(),
            session_id: frozen.session_id.clone(),
            turn_id: frozen.turn_id.clone(),
            generation_token: frozen.generation_token.clone(),
            task_run_id: frozen.task_run_id.clone(),
            action_class: frozen.action_class.clone(),
            normalized_action_kind: frozen.normalized_action_kind.clone(),
            argument_class: frozen.argument_class.clone(),
            argument_sha256: frozen.argument_sha256.clone(),
            canonical_resource: frozen.canonical_resource.clone(),
            scope_kind: selection.scope_kind,
            decision: selection.decision,
            issued_at_ms,
            expires_at_ms,
            nonce,
        };
        self.decisions
            .lock()
            .map_err(|_| {
                authority_error(
                    "shield_decision_store_unavailable",
                    "OOMU could not secure this decision.",
                )
            })?
            .insert(decision_id.clone(), decision);
        Ok(decision_id)
    }

    pub(crate) fn consume(
        &self,
        decision_id: &str,
        frozen: &FrozenShieldRequest,
        actor_id: &str,
    ) -> Result<NativeShieldDecision, NativeAuthorityError> {
        let decision_id = required("decision_id", decision_id)?;
        let actor_id = required("actor_id", actor_id)?;
        let decision = self
            .decisions
            .lock()
            .map_err(|_| {
                authority_error(
                    "shield_decision_store_unavailable",
                    "OOMU could not verify this decision.",
                )
            })?
            .remove(&decision_id)
            .ok_or_else(|| {
                authority_error(
                    "shield_decision_missing",
                    "This decision is no longer available.",
                )
            })?;
        let now = unix_time_ms_u64();
        if now > decision.expires_at_ms
            || decision.expires_at_ms.saturating_sub(decision.issued_at_ms) > SHIELD_DECISION_TTL_MS
        {
            return Err(authority_error(
                "shield_decision_expired",
                "This decision has expired.",
            ));
        }
        if decision.actor_id != actor_id
            || decision.approval_token != frozen.approval_token
            || decision.request_sha256 != frozen.request_sha256
            || decision.origin_kind != frozen.origin_kind
            || decision.session_id != frozen.session_id
            || decision.turn_id != frozen.turn_id
            || decision.generation_token != frozen.generation_token
            || decision.task_run_id != frozen.task_run_id
            || decision.action_class != frozen.action_class
            || decision.normalized_action_kind != frozen.normalized_action_kind
            || decision.argument_class != frozen.argument_class
            || decision.argument_sha256 != frozen.argument_sha256
            || decision.canonical_resource != frozen.canonical_resource
            || !frozen
                .permitted_scope_kinds
                .iter()
                .any(|kind| kind == &decision.scope_kind)
        {
            return Err(authority_error(
                "shield_decision_binding_mismatch",
                "This decision does not match the frozen action.",
            ));
        }
        Ok(decision)
    }
}

pub(crate) fn freeze_request(
    request: &ShieldApprovalRequest,
) -> Result<FrozenShieldRequest, NativeAuthorityError> {
    let approval_token = required("approval_token", &request.approval_token)?;
    let normalized_action_kind = required(
        "action_type",
        &request
            .action_type
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_"),
    )?;
    let action_class = required("action_class", &request.action_class)?;
    let argument_class = required("argument_class", &request.argument_class)?;
    validate_direct_chat_context(request)?;

    let mut permitted_scope_kinds = request
        .approval_scope_kinds
        .iter()
        .map(|kind| kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
        .collect::<Vec<_>>();
    permitted_scope_kinds.sort();
    permitted_scope_kinds.dedup();
    if !permitted_scope_kinds.iter().any(|kind| kind == "once")
        || permitted_scope_kinds.iter().any(|kind| {
            !matches!(
                kind.as_str(),
                "once" | "app_session" | "task" | "project_path" | "persistent"
            )
        })
        || (request.mandatory_reconfirm && permitted_scope_kinds.as_slice() != ["once".to_string()])
    {
        return Err(authority_error(
            "shield_request_scope_invalid",
            "This action has an invalid permission duration.",
        ));
    }

    let origin_kind = if request.turn_id.is_some() || request.generation_token.is_some() {
        "chat"
    } else if request.task_run_id.is_some() {
        "task"
    } else {
        "system"
    }
    .to_string();
    let canonical_resource = request
        .canonical_resource
        .as_deref()
        .or(request.target_path.as_deref())
        .unwrap_or(&normalized_action_kind)
        .trim()
        .to_string();
    if canonical_resource.is_empty() {
        return Err(authority_error(
            "shield_request_resource_invalid",
            "OOMU could not identify the exact destination for this action.",
        ));
    }
    let argument_sha256 = sha256_hex(
        serde_json::to_vec(&(
            &request.action_type,
            &request.preview,
            &request.target_path,
            &request.principal,
            &request.argument_class,
        ))
        .map_err(|_| {
            authority_error(
                "shield_request_serialization_failed",
                "OOMU could not freeze this action.",
            )
        })?
        .as_slice(),
    );
    let canonical = CanonicalShieldRequest {
        schema_version: SHIELD_DECISION_SCHEMA_VERSION,
        approval_token_sha256: sha256_hex(approval_token.as_bytes()),
        origin_kind: &origin_kind,
        session_id: &request.session_id,
        turn_id: &request.turn_id,
        generation_token: &request.generation_token,
        task_run_id: &request.task_run_id,
        project_id: &request.project_id,
        principal: &request.principal,
        normalized_action_kind: &normalized_action_kind,
        action_class: &action_class,
        argument_class: &argument_class,
        argument_sha256: &argument_sha256,
        canonical_resource: &canonical_resource,
        target_path: &request.target_path,
        mandatory_reconfirm: request.mandatory_reconfirm,
        permitted_scope_kinds: &permitted_scope_kinds,
    };
    let request_sha256 = sha256_hex(
        serde_json::to_vec(&canonical)
            .map_err(|_| {
                authority_error(
                    "shield_request_serialization_failed",
                    "OOMU could not freeze this action.",
                )
            })?
            .as_slice(),
    );
    Ok(FrozenShieldRequest {
        approval_token,
        request_sha256,
        origin_kind,
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        generation_token: request.generation_token.clone(),
        task_run_id: request.task_run_id.clone(),
        action_class,
        normalized_action_kind,
        argument_class,
        argument_sha256,
        canonical_resource,
        permitted_scope_kinds,
    })
}

fn validate_direct_chat_context(
    request: &ShieldApprovalRequest,
) -> Result<(), NativeAuthorityError> {
    let has_chat_context = request.session_id.is_some()
        || request.turn_id.is_some()
        || request.generation_token.is_some();
    if !has_chat_context {
        return Ok(());
    }
    for (field, value) in [
        ("session_id", request.session_id.as_deref()),
        ("turn_id", request.turn_id.as_deref()),
        ("generation_token", request.generation_token.as_deref()),
    ] {
        if value.is_none_or(|value| value.trim().is_empty()) {
            return Err(authority_error(
                "shield_request_origin_incomplete",
                &format!("Direct chat actions require immutable {field}."),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn request_native_selection(
    app: &tauri::AppHandle,
    request: &ShieldApprovalRequest,
    locale: &str,
    automated_approval: Option<crate::scenario_one_e2e_profile::NativeApprovalAutomation>,
) -> Result<NativeShieldPromptSelection, NativeAuthorityError> {
    let copy = prompt_copy(request, locale);
    let automated_scope = automated_approval
        .as_ref()
        .map(|approval| approval.scope().to_string());
    let selection = native_prompt(app, copy, automated_scope.clone()).await;
    #[cfg(debug_assertions)]
    if let Some(approval) = automated_approval.as_ref() {
        let approved = selection.as_ref().is_ok_and(|selection| {
            selection.decision == ShieldApprovalDecision::Approve
                && selection.scope_kind == approval.scope()
        });
        crate::scenario_one_e2e_profile::finish_automated_native_approval(approval, approved);
    }
    selection
}

struct ShieldPromptCopy {
    title: String,
    body: String,
    choices: Vec<(String, String)>,
    deny: String,
}

fn selection_for_prompt_response(
    response: isize,
    first_button_response: isize,
    choices: &[(String, String)],
) -> NativeShieldPromptSelection {
    response
        .checked_sub(first_button_response)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| choices.get(index))
        .map(|(scope_kind, _)| NativeShieldPromptSelection {
            decision: ShieldApprovalDecision::Approve,
            scope_kind: scope_kind.clone(),
        })
        .unwrap_or(NativeShieldPromptSelection {
            decision: ShieldApprovalDecision::Deny,
            scope_kind: "once".to_string(),
        })
}

fn prompt_copy(request: &ShieldApprovalRequest, locale: &str) -> ShieldPromptCopy {
    let explicit_place = request
        .canonical_resource
        .as_deref()
        .or(request.target_path.as_deref())
        .filter(|value| is_plain_location(value));
    let action = request.action_label.trim();
    let (title, wants, details, deny, device_location, path_preposition) = match locale {
        value if value.starts_with("de") => (
            "Diese Aktion erlauben?",
            "OOMU möchte",
            "Details",
            "Nicht erlauben",
            "auf diesem Mac",
            "in",
        ),
        value if value.starts_with("es") => (
            "¿Permitir esta acción?",
            "OOMU quiere",
            "Detalles",
            "No permitir",
            "en este Mac",
            "en",
        ),
        value if value.starts_with("fr") => (
            "Autoriser cette action ?",
            "OOMU souhaite",
            "Détails",
            "Ne pas autoriser",
            "sur ce Mac",
            "dans",
        ),
        value if value.starts_with("id") => (
            "Izinkan tindakan ini?",
            "OOMU ingin",
            "Detail",
            "Jangan Izinkan",
            "di Mac ini",
            "di",
        ),
        value if value.starts_with("ja") => (
            "この操作を許可しますか？",
            "OOMU が実行します:",
            "詳細",
            "許可しない",
            "このMac上で",
            "次の場所で",
        ),
        value if value.starts_with("pt") => (
            "Permitir esta ação?",
            "O OOMU quer",
            "Detalhes",
            "Não permitir",
            "neste Mac",
            "em",
        ),
        value if value.starts_with("ru") => (
            "Разрешить это действие?",
            "OOMU хочет",
            "Подробности",
            "Не разрешать",
            "на этом Mac",
            "в",
        ),
        value if value.starts_with("uk") => (
            "Дозволити цю дію?",
            "OOMU хоче",
            "Докладніше",
            "Не дозволяти",
            "на цьому Mac",
            "у",
        ),
        value if value.starts_with("vi") => (
            "Cho phép hành động này?",
            "OOMU muốn",
            "Chi tiết",
            "Không cho phép",
            "trên máy Mac này",
            "trong",
        ),
        value if value.starts_with("zh-TW") => (
            "允許此操作？",
            "OOMU 想要",
            "詳細資訊",
            "不允許",
            "在這台 Mac 上",
            "在",
        ),
        value if value.starts_with("zh") => (
            "允许此操作？",
            "OOMU 想要",
            "详细信息",
            "不允许",
            "在这台 Mac 上",
            "在",
        ),
        _ => (
            "Allow this action?",
            "OOMU wants to",
            "Details",
            "Don't Allow",
            "on this Mac",
            "in",
        ),
    };
    let location = explicit_place
        .map(|place| format!("{path_preposition} {place}"))
        .unwrap_or_else(|| device_location.to_string());
    let mut body = format!("{wants} {} {location}.", action.to_ascii_lowercase());
    if let Some(preview) = human_approval_preview(request, locale) {
        body.push_str(&format!("\n\n{details}:\n{preview}"));
    }
    let choices = request
        .approval_scope_kinds
        .iter()
        .map(|kind| (kind.clone(), scope_label(kind, locale)))
        .collect();
    ShieldPromptCopy {
        title: title.to_string(),
        body,
        choices,
        deny: deny.to_string(),
    }
}

#[derive(Clone, Copy)]
struct ApprovalPreviewLabels {
    calendar: &'static str,
    when: &'static str,
    title: &'static str,
    duration: &'static str,
    inputs: &'static str,
    creates: &'static str,
    web_research: &'static str,
    existing_files: &'static str,
    next_weekday: &'static str,
    minutes: &'static str,
    approved_files: &'static str,
    decision_pack_files: &'static str,
    official_sources: &'static str,
    kept: &'static str,
}

#[rustfmt::skip]
fn approval_preview_labels(locale: &str) -> ApprovalPreviewLabels {
    match locale {
        value if value.starts_with("de") => ApprovalPreviewLabels { calendar: "Kalender", when: "Wann", title: "Titel", duration: "Dauer", inputs: "Eingaben", creates: "Erstellt", web_research: "Web-Recherche", existing_files: "Vorhandene Dateien", next_weekday: "nächster Werktag", minutes: "Minuten", approved_files: "freigegebene Dateien", decision_pack_files: "neue Entscheidungspaket-Dateien", official_sources: "offizielle öffentliche Quellen", kept: "bleiben erhalten" },
        value if value.starts_with("es") => ApprovalPreviewLabels { calendar: "Calendario", when: "Cuándo", title: "Título", duration: "Duración", inputs: "Entradas", creates: "Crea", web_research: "Investigación web", existing_files: "Archivos existentes", next_weekday: "próximo día laborable", minutes: "minutos", approved_files: "archivos aprobados", decision_pack_files: "archivos nuevos del paquete de decisión", official_sources: "fuentes públicas oficiales", kept: "se conservan" },
        value if value.starts_with("fr") => ApprovalPreviewLabels { calendar: "Calendrier", when: "Quand", title: "Titre", duration: "Durée", inputs: "Entrées", creates: "Crée", web_research: "Recherche web", existing_files: "Fichiers existants", next_weekday: "prochain jour ouvré", minutes: "minutes", approved_files: "fichiers approuvés", decision_pack_files: "nouveaux fichiers du dossier de décision", official_sources: "sources publiques officielles", kept: "conservés" },
        value if value.starts_with("id") => ApprovalPreviewLabels { calendar: "Kalender", when: "Waktu", title: "Judul", duration: "Durasi", inputs: "Masukan", creates: "Membuat", web_research: "Riset web", existing_files: "File yang ada", next_weekday: "hari kerja berikutnya", minutes: "menit", approved_files: "file yang disetujui", decision_pack_files: "file paket keputusan baru", official_sources: "sumber publik resmi", kept: "tetap disimpan" },
        value if value.starts_with("ja") => ApprovalPreviewLabels { calendar: "カレンダー", when: "日時", title: "タイトル", duration: "所要時間", inputs: "入力", creates: "作成", web_research: "ウェブ調査", existing_files: "既存ファイル", next_weekday: "次の平日", minutes: "分", approved_files: "承認済みファイル", decision_pack_files: "新しい意思決定パックファイル", official_sources: "公式の公開情報", kept: "保持されます" },
        value if value.starts_with("pt") => ApprovalPreviewLabels { calendar: "Calendário", when: "Quando", title: "Título", duration: "Duração", inputs: "Entradas", creates: "Cria", web_research: "Pesquisa na web", existing_files: "Arquivos existentes", next_weekday: "próximo dia útil", minutes: "minutos", approved_files: "arquivos aprovados", decision_pack_files: "novos arquivos do pacote de decisão", official_sources: "fontes públicas oficiais", kept: "preservados" },
        value if value.starts_with("ru") => ApprovalPreviewLabels { calendar: "Календарь", when: "Когда", title: "Название", duration: "Длительность", inputs: "Исходные файлы", creates: "Создаёт", web_research: "Поиск в интернете", existing_files: "Существующие файлы", next_weekday: "следующий рабочий день", minutes: "минут", approved_files: "одобренных файлов", decision_pack_files: "новых файлов пакета решений", official_sources: "официальные открытые источники", kept: "сохраняются" },
        value if value.starts_with("uk") => ApprovalPreviewLabels { calendar: "Календар", when: "Коли", title: "Назва", duration: "Тривалість", inputs: "Вхідні файли", creates: "Створює", web_research: "Пошук в інтернеті", existing_files: "Наявні файли", next_weekday: "наступний робочий день", minutes: "хвилин", approved_files: "схвалених файлів", decision_pack_files: "нових файлів пакета рішень", official_sources: "офіційні відкриті джерела", kept: "зберігаються" },
        value if value.starts_with("vi") => ApprovalPreviewLabels { calendar: "Lịch", when: "Thời gian", title: "Tiêu đề", duration: "Thời lượng", inputs: "Tệp đầu vào", creates: "Tạo", web_research: "Nghiên cứu web", existing_files: "Tệp hiện có", next_weekday: "ngày làm việc tiếp theo", minutes: "phút", approved_files: "tệp đã phê duyệt", decision_pack_files: "tệp gói quyết định mới", official_sources: "nguồn công khai chính thức", kept: "được giữ nguyên" },
        value if value.starts_with("zh-TW") => ApprovalPreviewLabels { calendar: "行事曆", when: "時間", title: "標題", duration: "時長", inputs: "輸入", creates: "建立", web_research: "網路研究", existing_files: "現有檔案", next_weekday: "下一個工作日", minutes: "分鐘", approved_files: "個已核准檔案", decision_pack_files: "個新的決策包檔案", official_sources: "官方公開來源", kept: "保留" },
        value if value.starts_with("zh") => ApprovalPreviewLabels { calendar: "日历", when: "时间", title: "标题", duration: "时长", inputs: "输入", creates: "创建", web_research: "网络研究", existing_files: "现有文件", next_weekday: "下一个工作日", minutes: "分钟", approved_files: "个已批准文件", decision_pack_files: "个新的决策包文件", official_sources: "官方公开来源", kept: "保留" },
        _ => ApprovalPreviewLabels { calendar: "Calendar", when: "When", title: "Title", duration: "Duration", inputs: "Inputs", creates: "Creates", web_research: "Web research", existing_files: "Existing files", next_weekday: "next weekday", minutes: "minutes", approved_files: "approved files", decision_pack_files: "new decision-pack files", official_sources: "official public sources", kept: "kept" },
    }
}

fn human_approval_preview(request: &ShieldApprovalRequest, locale: &str) -> Option<String> {
    let labels = approval_preview_labels(locale);
    let action_type = request
        .action_type
        .trim()
        .replace('-', "_")
        .to_ascii_lowercase();
    let preview = serde_json::from_str::<Value>(request.preview.trim()).ok();

    if matches!(
        action_type.as_str(),
        "create_conflict_free_calendar_event" | "create_system_calendar_event"
    ) {
        if let Some(value) = preview.as_ref() {
            let calendar = value
                .get("calendarName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| is_plain_approval_text(value));
            let title = value
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| is_plain_approval_text(value));
            let start = value
                .get("windowStartLocal")
                .or_else(|| value.get("startDate"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| is_plain_approval_text(value));
            let end = value
                .get("windowEndLocal")
                .or_else(|| value.get("endDate"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| is_plain_approval_text(value));
            let day = value
                .get("day")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| is_plain_approval_text(value));
            let duration = value.get("durationMinutes").and_then(Value::as_u64);
            if let (Some(calendar), Some(title), Some(start), Some(end)) =
                (calendar, title, start, end)
            {
                let when = match day {
                    Some("next_weekday") => format!("{}, {start}–{end}", labels.next_weekday),
                    Some(day) if !day.is_empty() => format!("{day}, {start}–{end}"),
                    _ => format!("{start}–{end}"),
                };
                let mut rows = vec![
                    format!("{}: {calendar}", labels.calendar),
                    format!("{}: {when}", labels.when),
                    format!("{}: {title}", labels.title),
                ];
                if let Some(duration) = duration {
                    rows.push(format!(
                        "{}: {duration} {}",
                        labels.duration, labels.minutes
                    ));
                }
                return Some(rows.join("\n"));
            }
        }
    }

    if action_type == "create_system_calendar" {
        if let Some(calendar) = preview
            .as_ref()
            .and_then(|value| value.get("calendarName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| is_plain_approval_text(value))
        {
            return Some(format!("{}: {calendar}", labels.calendar));
        }
    }

    if action_type == "create_decision_pack" {
        if let Some(value) = preview.as_ref() {
            let input_count = value
                .get("inputPaths")
                .and_then(Value::as_array)
                .map(Vec::len);
            let output_count = value
                .get("outputs")
                .and_then(Value::as_object)
                .map(serde_json::Map::len);
            let has_research = value.get("researchPolicy").is_some_and(Value::is_object);
            let preserves_existing =
                value.get("willOverwrite").and_then(Value::as_bool) == Some(false);
            if let (Some(input_count), Some(output_count), true, true) =
                (input_count, output_count, has_research, preserves_existing)
            {
                return Some(
                    [
                        format!("{}: {input_count} {}", labels.inputs, labels.approved_files),
                        format!(
                            "{}: {output_count} {}",
                            labels.creates, labels.decision_pack_files
                        ),
                        format!("{}: {}", labels.web_research, labels.official_sources),
                        format!("{}: {}", labels.existing_files, labels.kept),
                    ]
                    .join("\n"),
                );
            }
        }
    }

    [
        request.semantic_summary.trim(),
        request.semantic_detail.trim(),
    ]
    .into_iter()
    .find(|value| is_plain_approval_text(value))
    .map(str::to_string)
    .or_else(|| {
        let action = request.action_label.trim();
        is_plain_approval_text(action).then(|| format!("{action}."))
    })
}

fn is_plain_approval_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2_000
        && !value.contains('{')
        && !value.contains('[')
        && !value.contains("\":")
        && !value.contains("researchPolicy")
        && !value.contains("imported_")
        && !value
            .chars()
            .zip(value.chars().skip(1))
            .any(|(left, right)| left.is_lowercase() && right.is_uppercase())
        && !value.chars().any(char::is_control)
}

fn is_plain_location(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with('/') || value.starts_with("~/"))
        && !value.contains('{')
        && !value.contains('[')
        && !value.contains("imported_")
        && !value.chars().any(char::is_control)
}

fn scope_label(kind: &str, locale: &str) -> String {
    let english = match kind {
        "app_session" => "For This Session",
        "task" => "For This Task",
        "project_path" => "For This Project",
        "persistent" => "Always Allow",
        _ => "Allow Once",
    };
    match locale {
        value if value.starts_with("de") => match kind {
            "app_session" => "Für diese Sitzung",
            "task" => "Für diese Aufgabe",
            "project_path" => "Für dieses Projekt",
            "persistent" => "Immer erlauben",
            _ => "Einmal erlauben",
        },
        value if value.starts_with("es") => match kind {
            "app_session" => "Durante esta sesión",
            "task" => "Para esta tarea",
            "project_path" => "Para este proyecto",
            "persistent" => "Permitir siempre",
            _ => "Permitir una vez",
        },
        value if value.starts_with("fr") => match kind {
            "app_session" => "Pour cette session",
            "task" => "Pour cette tâche",
            "project_path" => "Pour ce projet",
            "persistent" => "Toujours autoriser",
            _ => "Autoriser une fois",
        },
        value if value.starts_with("id") => match kind {
            "app_session" => "Untuk Sesi Ini",
            "task" => "Untuk Tugas Ini",
            "project_path" => "Untuk Proyek Ini",
            "persistent" => "Selalu Izinkan",
            _ => "Izinkan Sekali",
        },
        value if value.starts_with("ja") => match kind {
            "app_session" => "このセッション中",
            "task" => "このタスク中",
            "project_path" => "このプロジェクト中",
            "persistent" => "常に許可",
            _ => "1回だけ許可",
        },
        value if value.starts_with("pt") => match kind {
            "app_session" => "Nesta sessão",
            "task" => "Nesta tarefa",
            "project_path" => "Neste projeto",
            "persistent" => "Permitir sempre",
            _ => "Permitir uma vez",
        },
        value if value.starts_with("ru") => match kind {
            "app_session" => "Для этого сеанса",
            "task" => "Для этой задачи",
            "project_path" => "Для этого проекта",
            "persistent" => "Разрешать всегда",
            _ => "Разрешить один раз",
        },
        value if value.starts_with("uk") => match kind {
            "app_session" => "Для цього сеансу",
            "task" => "Для цього завдання",
            "project_path" => "Для цього проєкту",
            "persistent" => "Завжди дозволяти",
            _ => "Дозволити один раз",
        },
        value if value.starts_with("vi") => match kind {
            "app_session" => "Trong phiên này",
            "task" => "Cho tác vụ này",
            "project_path" => "Cho dự án này",
            "persistent" => "Luôn cho phép",
            _ => "Chỉ cho phép một lần",
        },
        value if value.starts_with("zh-TW") => match kind {
            "app_session" => "本次工作階段",
            "task" => "本次任務",
            "project_path" => "本專案",
            "persistent" => "永遠允許",
            _ => "僅允許一次",
        },
        value if value.starts_with("zh") => match kind {
            "app_session" => "本次会话",
            "task" => "本次任务",
            "project_path" => "本项目",
            "persistent" => "始终允许",
            _ => "仅允许一次",
        },
        _ => english,
    }
    .to_string()
}

#[cfg(target_os = "macos")]
async fn native_prompt(
    app: &tauri::AppHandle,
    copy: ShieldPromptCopy,
    automated_scope: Option<String>,
) -> Result<NativeShieldPromptSelection, NativeAuthorityError> {
    use block2::RcBlock;
    use objc2::{rc::autoreleasepool, sel, MainThreadMarker};
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSWindow};
    use objc2_foundation::{NSObjectNSDelayedPerforming, NSString};
    use std::cell::RefCell;
    use tauri::Manager;
    use tokio::sync::oneshot;

    let main_window = app.get_webview_window("main").ok_or_else(|| {
        authority_error(
            "shield_native_prompt_window_unavailable",
            "OOMU could not attach the permission prompt to its window.",
        )
    })?;
    main_window.unminimize().map_err(|_| {
        authority_error(
            "shield_native_prompt_window_unavailable",
            "OOMU could not restore its window for the permission prompt.",
        )
    })?;
    main_window.show().map_err(|_| {
        authority_error(
            "shield_native_prompt_window_unavailable",
            "OOMU could not show its window for the permission prompt.",
        )
    })?;
    main_window.set_focus().map_err(|_| {
        authority_error(
            "shield_native_prompt_window_unavailable",
            "OOMU could not focus its window for the permission prompt.",
        )
    })?;

    let (sender, receiver) = oneshot::channel();
    app.run_on_main_thread(move || {
        autoreleasepool(|_| {
            let Some(mtm) = MainThreadMarker::new() else {
                let _ = sender.send(None);
                return;
            };
            let parent_window = main_window.ns_window().ok().and_then(|pointer| {
                // SAFETY: Tauri returns the retained AppKit window backing
                // `main_window`, which remains owned by the moved webview
                // handle for the duration of this main-thread closure.
                unsafe { objc2::rc::Retained::<NSWindow>::retain(pointer.cast()) }
            });
            let Some(parent_window) = parent_window else {
                let _ = sender.send(None);
                return;
            };
            let application = NSApplication::sharedApplication(mtm);
            #[allow(deprecated)]
            application.activateIgnoringOtherApps(true);
            parent_window.makeKeyAndOrderFront(None);

            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(&NSString::from_str(&copy.title));
            alert.setInformativeText(&NSString::from_str(&copy.body));
            let mut automated_button = None;
            for (scope, label) in &copy.choices {
                let button = alert.addButtonWithTitle(&NSString::from_str(label));
                if automated_scope.as_deref() == Some(scope.as_str()) {
                    automated_button = Some(button);
                }
            }
            let deny_button = alert.addButtonWithTitle(&NSString::from_str(&copy.deny));
            deny_button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            if let (Some(scope), Some(button)) = (automated_scope.as_deref(), automated_button) {
                // SAFETY: `performClick:` is an NSControl selector and
                // `button` is the actual retained NSAlert button. A sheet
                // runs on AppKit's normal main run loop.
                unsafe {
                    button.performSelector_withObject_afterDelay(sel!(performClick:), None, 0.35);
                }
                eprintln!(
                    "OOMU_SCENARIO_ONE_E2E_TRACE stage=native_shield status=scheduled_real_button scope={scope}"
                );
            }

            // Keep the alert alive through the asynchronous sheet and use a
            // heap-retained completion block. There is intentionally no
            // second `runModal` call: one click yields one AppKit response.
            let retained_alert = RefCell::new(Some(alert.clone()));
            let choices = copy.choices;
            let sender = RefCell::new(Some(sender));
            let completion = RcBlock::new(move |response| {
                let selection =
                    selection_for_prompt_response(response, NSAlertFirstButtonReturn, &choices);
                if let Some(sender) = sender.borrow_mut().take() {
                    let _ = sender.send(Some(selection));
                }
                retained_alert.borrow_mut().take();
            });
            alert.beginSheetModalForWindow_completionHandler(
                &parent_window,
                Some(&completion),
            );
        });
    })
    .map_err(|_| {
        authority_error(
            "shield_native_prompt_failed",
            "OOMU could not present the native permission prompt.",
        )
    })?;
    receiver
        .await
        .map_err(|_| {
            authority_error(
                "shield_native_prompt_closed",
                "The native permission prompt closed without a decision.",
            )
        })?
        .ok_or_else(|| {
            authority_error(
                "shield_native_prompt_failed",
                "OOMU could not present the native permission prompt.",
            )
        })
}

#[cfg(not(target_os = "macos"))]
async fn native_prompt(
    _app: &tauri::AppHandle,
    _copy: ShieldPromptCopy,
    _automated_scope: Option<String>,
) -> Result<NativeShieldPromptSelection, NativeAuthorityError> {
    Err(authority_error(
        "shield_native_prompt_unavailable",
        "Native Shield approval is unavailable on this platform.",
    ))
}

fn required(field: &str, value: &str) -> Result<String, NativeAuthorityError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(authority_error(
            "shield_decision_field_missing",
            &format!("Shield decision field {field} is required."),
        ));
    }
    Ok(value.to_string())
}

fn random_token() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ShieldApprovalRequest {
        ShieldApprovalRequest {
            approval_token: "approval-secret".into(),
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            generation_token: Some("generation-1".into()),
            action_type: "shell_command".into(),
            action_label: "Run a local command".into(),
            target_path: None,
            principal: Some("actor-1".into()),
            risk_tier: "critical".into(),
            reason: "Exact approval required.".into(),
            estimated_token_costs: None,
            requested_at_ms: unix_time_ms_u64(),
            preview: "printf canary".into(),
            semantic_summary: "Run one command".into(),
            semantic_detail: "Run the reviewed command".into(),
            approval_tier: "explicit_confirmation".into(),
            approval_mode: "explicit_confirmation".into(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: None,
            task_run_id: None,
            action_class: "shell_command".into(),
            argument_class: "shell_command:small".into(),
            canonical_resource: Some("shell:printf-canary".into()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".into()],
        }
    }

    #[test]
    fn shield_native_decision_is_single_use_and_exactly_bound() {
        let store = NativeShieldDecisionStore::default();
        let frozen = freeze_request(&request()).unwrap();
        let decision_id = store
            .issue_after_native_presence(
                &frozen,
                "actor-1".into(),
                NativeShieldPromptSelection {
                    decision: ShieldApprovalDecision::Approve,
                    scope_kind: "once".into(),
                },
            )
            .unwrap();
        let decision = store.consume(&decision_id, &frozen, "actor-1").unwrap();
        assert_eq!(decision.decision, ShieldApprovalDecision::Approve);
        assert_eq!(decision.scope_kind, "once");
        assert!(!decision.nonce.is_empty());
        assert_eq!(decision.decision_id, decision_id);
        assert!(store.consume(&decision_id, &frozen, "actor-1").is_err());
    }

    #[test]
    fn shield_native_decision_rejects_actor_and_request_mutation() {
        let store = NativeShieldDecisionStore::default();
        let original = freeze_request(&request()).unwrap();
        let decision_id = store
            .issue_after_native_presence(
                &original,
                "actor-1".into(),
                NativeShieldPromptSelection {
                    decision: ShieldApprovalDecision::Approve,
                    scope_kind: "once".into(),
                },
            )
            .unwrap();
        let mut changed = request();
        changed.preview = "printf changed".into();
        let changed = freeze_request(&changed).unwrap();
        assert!(store.consume(&decision_id, &changed, "actor-1").is_err());

        let second = store
            .issue_after_native_presence(
                &original,
                "actor-1".into(),
                NativeShieldPromptSelection {
                    decision: ShieldApprovalDecision::Approve,
                    scope_kind: "once".into(),
                },
            )
            .unwrap();
        assert!(store.consume(&second, &original, "actor-2").is_err());
    }

    #[test]
    fn shield_native_decision_rejects_partial_chat_context() {
        let mut partial = request();
        partial.generation_token = None;
        assert_eq!(
            freeze_request(&partial).unwrap_err().code,
            "shield_request_origin_incomplete"
        );
    }

    #[test]
    fn native_prompt_response_fails_closed_for_cancel_and_unknown_codes() {
        let choices = vec![
            ("once".to_string(), "Allow Once".to_string()),
            ("app_session".to_string(), "For This Session".to_string()),
        ];
        let first = 1_000_isize;
        let approved = selection_for_prompt_response(first + 1, first, &choices);
        assert_eq!(approved.decision, ShieldApprovalDecision::Approve);
        assert_eq!(approved.scope_kind, "app_session");

        for response in [first - 1, -1, 0, first + choices.len() as isize, isize::MAX] {
            let selection = selection_for_prompt_response(response, first, &choices);
            assert_eq!(selection.decision, ShieldApprovalDecision::Deny);
            assert_eq!(selection.scope_kind, "once");
        }
    }

    #[test]
    fn native_scope_labels_preserve_each_grant_in_every_supported_locale() {
        for locale in [
            "de-DE", "en-US", "es-ES", "fr-FR", "id-ID", "ja-JP", "pt-BR", "ru-RU", "uk-UA",
            "vi-VN", "zh-CN", "zh-TW",
        ] {
            let labels = ["once", "app_session", "task", "project_path", "persistent"]
                .map(|kind| scope_label(kind, locale));
            let unique = labels.iter().collect::<std::collections::HashSet<_>>();
            assert_eq!(unique.len(), labels.len(), "{locale}");
        }
    }

    #[test]
    fn native_calendar_prompt_uses_human_details_and_device_grammar() {
        let mut calendar = request();
        calendar.action_type = "create_conflict_free_calendar_event".into();
        calendar.action_label = "Find a time and add a Calendar event".into();
        calendar.canonical_resource = None;
        calendar.preview = serde_json::json!({
            "availability": "tentative",
            "calendarName": "OOMU Test",
            "day": "next_weekday",
            "durationMinutes": 60,
            "location": "",
            "title": "Supplier Decision Review",
            "windowEndLocal": "16:00",
            "windowStartLocal": "13:00"
        })
        .to_string();

        let copy = prompt_copy(&calendar, "en-US");
        assert!(copy.body.contains("on this Mac."));
        assert!(!copy.body.contains("in this Mac"));
        assert!(copy.body.contains("Calendar: OOMU Test"));
        assert!(copy.body.contains("When: next weekday, 13:00–16:00"));
        assert!(copy.body.contains("Title: Supplier Decision Review"));
        assert!(!copy.body.contains('{'));
        assert!(!copy.body.contains("\":"));
        assert!(!copy.body.contains("windowStartLocal"));
    }

    #[test]
    fn native_decision_pack_prompt_summarizes_counts_without_payload_or_paths() {
        let mut decision_pack = request();
        decision_pack.action_type = "create_decision_pack".into();
        decision_pack.action_label = "Create a supplier decision pack".into();
        decision_pack.canonical_resource = Some("/Users/test/Decision Pack".into());
        decision_pack.preview = serde_json::json!({
            "action": "create_decision_pack",
            "calendarOrMailIncluded": false,
            "inputPaths": [
                "/Users/test/imported_1783703954441/supplier_proposals.json",
                "/Users/test/q3_strategic_vendor_proposals.txt"
            ],
            "outputDirectory": "/Users/test/Decision Pack",
            "outputs": {
                "brief": "/Users/test/Decision Pack/brief.docx",
                "memo": "/Users/test/Decision Pack/memo.md",
                "slides": "/Users/test/Decision Pack/review.pptx",
                "workbook": "/Users/test/Decision Pack/analysis.xlsx"
            },
            "researchPolicy": { "subjects": [] },
            "willOverwrite": false
        })
        .to_string();

        let copy = prompt_copy(&decision_pack, "en-US");
        let details = copy
            .body
            .split("\n\nDetails:\n")
            .nth(1)
            .expect("details block");
        assert!(details.contains("Inputs: 2 approved files"));
        assert!(details.contains("Creates: 4 new decision-pack files"));
        assert!(details.contains("Web research: official public sources"));
        assert!(!details.contains('{'));
        assert!(!details.contains("\":"));
        assert!(!details.contains("researchPolicy"));
        assert!(!details.contains("imported_"));
        assert!(!details.contains("/Users/"));
    }

    #[test]
    fn native_prompt_never_falls_back_to_a_machine_preview() {
        let mut generic = request();
        generic.preview = r#"{"internalId":"imported_1783703954441","willRun":true}"#.into();
        generic.semantic_summary = "Run the reviewed command.".into();

        let copy = prompt_copy(&generic, "en-US");
        assert!(copy.body.contains("Run the reviewed command."));
        assert!(!copy.body.contains("internalId"));
        assert!(!copy.body.contains("imported_"));
        assert!(!copy.body.contains('{'));
    }
}
