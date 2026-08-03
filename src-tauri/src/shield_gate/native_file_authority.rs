use super::*;
use crate::sovereign_identity::{
    NativeFileAuthorityClaim, NativeFileAuthorityEnvelope, NATIVE_FILE_AUTHORITY_VERSION,
};
use std::sync::Mutex;

const AUTHORITY_TTL_MS: i64 = 60_000;
static CONSUMED_AUTHORITIES: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeFileAccessKind {
    FileRead,
    FileList,
}

impl NativeFileAccessKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::FileRead => "file_read",
            Self::FileList => "file_list",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeFileAccessAction {
    pub kind: NativeFileAccessKind,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeDirectFileAccessRequest {
    pub action: NativeFileAccessAction,
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
}

impl NativeDirectFileAccessRequest {
    pub(super) fn into_observed_command(
        self,
        persistence: &PersistenceEngine,
    ) -> Result<ExecuteCommandRequest, ShieldGateError> {
        let session_id = required("session_id", Some(&self.session_id))?.to_string();
        let turn_id = required("turn_id", Some(&self.turn_id))?.to_string();
        let generation_token =
            required("generation_token", Some(&self.generation_token))?.to_string();
        let context = persistence
            .select_chat_turn_context(&turn_id)
            .map_err(observed_turn_error)?
            .ok_or_else(|| observed_turn_error("The accepted turn does not exist."))?;
        if context.session_id != session_id || context.generation_token != generation_token {
            return Err(observed_turn_error(
                "The accepted turn no longer matches this file request.",
            ));
        }
        persistence
            .validate_accepted_chat_turn_generation(&context)
            .map_err(observed_turn_error)?;
        let session = persistence
            .select_chat_session_by_id(&context.session_id)
            .map_err(observed_turn_error)?;
        Ok(ExecuteCommandRequest {
            action: RequestedAction {
                kind: self.action.kind.as_str().to_string(),
                principal: Some(context.agent_id.clone()),
                path: Some(self.action.path),
                content: None,
            },
            logical_certificate: None,
            session_id: Some(context.session_id),
            turn_id: Some(context.turn_id),
            generation_token: Some(context.generation_token),
            agent_id: Some(context.agent_id),
            provider_id: Some(context.provider_id),
            model_id: Some(context.model_id),
            parent_turn_id: context.parent_turn_id,
            root_turn_id: Some(context.root_turn_id),
            turn_kind: Some(context.turn_kind),
            project_id: session.project_id,
            task_run_id: None,
        })
    }
}

fn observed_turn_error(error: impl std::fmt::Display) -> ShieldGateError {
    security_boundary_violation(format!(
        "Native file access requires one durable accepted turn. {error}"
    ))
}

fn required<'a>(field: &str, value: Option<&'a str>) -> Result<&'a str, ShieldGateError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            security_boundary_violation(format!("Native file access requires immutable {field}."))
        })
}

fn canonical_target_and_root(path: &str) -> Result<(PathBuf, PathBuf), ShieldGateError> {
    let requested = expand_shield_home_path(path, "native file access")?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "Native file access rejected path traversal.".to_string(),
        ));
    }
    let target = fs::canonicalize(&requested).map_err(|_| {
        security_boundary_violation("The requested local target does not exist.".to_string())
    })?;
    let metadata = fs::metadata(&target).map_err(|_| {
        security_boundary_violation("The requested local target is unavailable.".to_string())
    })?;
    let root = if metadata.is_dir() {
        target.clone()
    } else {
        target.parent().map(Path::to_path_buf).ok_or_else(|| {
            security_boundary_violation(
                "The requested local target has no valid containing directory.".to_string(),
            )
        })?
    };
    Ok((target, root))
}

pub(super) fn is_native_file_access_kind(action_kind: &str) -> bool {
    matches!(
        super::normalize_action_kind(action_kind).as_str(),
        "file_read" | "file_list"
    )
}

pub(super) fn issue(
    request: &mut ExecuteCommandRequest,
    identity: &SovereignIdentity,
) -> Result<NativeFileAuthorityEnvelope, ShieldGateError> {
    if !is_native_file_access_kind(&request.action.kind) {
        return Err(security_boundary_violation(
            "Native file authority is limited to read-only file access.".to_string(),
        ));
    }
    if request.logical_certificate.is_some() {
        return Err(security_boundary_violation(
            "Renderer-supplied certificates are not accepted for native file access.".to_string(),
        ));
    }
    let requested_path = required("path", request.action.path.as_deref())?;
    let (target, root) = canonical_target_and_root(requested_path)?;
    request.action.path = Some(target.display().to_string());
    let issued_at_ms = unix_time_ms_i64();
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let claim = NativeFileAuthorityClaim {
        version: NATIVE_FILE_AUTHORITY_VERSION,
        nonce: hex::encode(nonce),
        action_kind: super::normalize_action_kind(&request.action.kind),
        canonical_target: target.display().to_string(),
        canonical_root: root.display().to_string(),
        session_id: required("session_id", request.session_id.as_deref())?.to_string(),
        turn_id: required("turn_id", request.turn_id.as_deref())?.to_string(),
        generation_token: required("generation_token", request.generation_token.as_deref())?
            .to_string(),
        agent_id: required("agent_id", request.agent_id.as_deref())?.to_string(),
        issued_at_ms,
        expires_at_ms: issued_at_ms.saturating_add(AUTHORITY_TTL_MS),
    };
    identity
        .sign_native_file_authority(claim)
        .map_err(|error| ShieldGateError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })
}

pub(super) fn consume(
    request: &ExecuteCommandRequest,
    envelope: &NativeFileAuthorityEnvelope,
    identity: &SovereignIdentity,
) -> Result<(), ShieldGateError> {
    identity
        .verify_native_file_authority(envelope)
        .map_err(|error| ShieldGateError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })?;
    let claim = &envelope.claim;
    let now_ms = unix_time_ms_i64();
    let requested_path = required("path", request.action.path.as_deref())?;
    let (target, root) = canonical_target_and_root(requested_path)?;
    let binding_matches = claim.version == NATIVE_FILE_AUTHORITY_VERSION
        && claim.action_kind == super::normalize_action_kind(&request.action.kind)
        && claim.canonical_target == target.display().to_string()
        && claim.canonical_root == root.display().to_string()
        && claim.session_id == required("session_id", request.session_id.as_deref())?
        && claim.turn_id == required("turn_id", request.turn_id.as_deref())?
        && claim.generation_token
            == required("generation_token", request.generation_token.as_deref())?
        && claim.agent_id == required("agent_id", request.agent_id.as_deref())?
        && claim.issued_at_ms <= now_ms.saturating_add(5_000)
        && claim.expires_at_ms >= now_ms
        && claim.expires_at_ms.saturating_sub(claim.issued_at_ms) <= AUTHORITY_TTL_MS;
    if !binding_matches {
        return Err(security_boundary_violation(
            "Native file authority no longer matches the accepted operation.".to_string(),
        ));
    }

    let mut consumed = CONSUMED_AUTHORITIES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| {
            security_boundary_violation("Native file authority state is unavailable.".to_string())
        })?;
    consumed.retain(|_, expires_at_ms| *expires_at_ms >= now_ms);
    if consumed
        .insert(claim.nonce.clone(), claim.expires_at_ms)
        .is_some()
    {
        return Err(security_boundary_violation(
            "Native file authority has already been consumed.".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn reject_renderer_certificate(
    certificate: Option<&LogicalCertificate>,
) -> Result<(), ShieldGateError> {
    if certificate.is_some() {
        return Err(security_boundary_violation(
            "Renderer-supplied certificates are not accepted for native file access.".to_string(),
        ));
    }
    Ok(())
}

struct ApprovedReceiptContext {
    canonical_path: PathBuf,
    metadata: fs::Metadata,
    target_is_directory: bool,
    display_name: String,
    target_identity_hash: String,
    session_id: String,
    issued_turn_id: String,
    root_turn_id: String,
    agent_id: String,
    display_message: String,
}

fn build_approved_receipt(
    context: ApprovedReceiptContext,
    content: String,
    identity: &SovereignIdentity,
) -> Result<PrepareApprovedChatFileResponse, ShieldGateError> {
    let mime_type =
        approved_chat_read_mime_type(&context.canonical_path, context.target_is_directory);
    let media_bytes = if !context.target_is_directory && mime_type.starts_with("image/") {
        let file = open_bound_external_target(
            &context.canonical_path,
            approved_file_identity(&context.metadata),
            false,
        )
        .map_err(approved_chat_file_error)?;
        let mut bytes = Vec::with_capacity(
            (context.metadata.len() as usize).min(MAX_APPROVED_CHAT_FILE_MEDIA_BYTES),
        );
        file.take((MAX_APPROVED_CHAT_FILE_MEDIA_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                approved_chat_file_error("The approved image could not be read safely.")
            })?;
        if bytes.is_empty() || bytes.len() > MAX_APPROVED_CHAT_FILE_MEDIA_BYTES {
            return Err(approved_chat_file_error(
                "The approved image is too large to use in this chat.",
            ));
        }
        crate::tools::vision::validate_visual_dimensions(&bytes, &mime_type)
            .map_err(approved_chat_file_error)?;
        Some(Zeroizing::new(bytes))
    } else {
        None
    };
    let issued_at_ms = unix_time_ms_i64();
    let mut receipt_random = [0u8; 24];
    OsRng.fill_bytes(&mut receipt_random);
    let receipt_id = hex::encode(receipt_random);
    let media_sha256 = media_bytes
        .as_ref()
        .map(|bytes| sha256_hex(bytes.as_slice()));
    let byte_count = approved_chat_read_byte_count(
        &context.metadata,
        &content,
        media_bytes.as_ref(),
        context.target_is_directory,
    );
    let payload = ApprovedFileReceiptPayload {
        version: APPROVED_CHAT_FILE_RECEIPT_VERSION,
        receipt_id: receipt_id.clone(),
        session_id: context.session_id.clone(),
        issued_turn_id: context.issued_turn_id,
        root_turn_id: context.root_turn_id.clone(),
        agent_id: context.agent_id.clone(),
        target_identity_hash: context.target_identity_hash,
        display_name: context.display_name.clone(),
        mime_type: mime_type.clone(),
        byte_count,
        content_sha256: sha256_hex(content.as_bytes()),
        content,
        media_sha256: media_sha256.clone(),
        display_message: context.display_message,
        issued_at_ms,
        expires_at_ms: issued_at_ms.saturating_add(APPROVED_CHAT_FILE_RECEIPT_TTL_MS),
    };
    let payload_json = serde_json::to_string(&payload)
        .map_err(|_| approved_chat_file_error("The file receipt could not be created."))?;
    let signature = identity
        .sign_payload(&payload_json)
        .map_err(|_| approved_chat_file_error("The file receipt could not be signed."))?;
    if let (Some(bytes), Some(sha256)) = (media_bytes, media_sha256) {
        cache_approved_chat_file_media(
            receipt_id,
            ApprovedChatFileMedia {
                session_id: context.session_id,
                root_turn_id: context.root_turn_id,
                agent_id: context.agent_id,
                mime_type,
                sha256,
                issued_at_ms,
                expires_at_ms: payload.expires_at_ms,
                bytes,
            },
        )?;
    }
    Ok(PrepareApprovedChatFileResponse {
        display_name: context.display_name,
        mime_type: payload.mime_type.clone(),
        byte_count: payload.byte_count,
        receipt: ApprovedFileReceiptToken {
            payload: URL_SAFE_NO_PAD.encode(payload_json.as_bytes()),
            signature,
        },
    })
}

#[tauri::command]
pub async fn prepare_approved_chat_file(
    request: PrepareApprovedChatFileRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<PrepareApprovedChatFileResponse, ShieldGateError> {
    let PrepareApprovedChatFileRequest {
        access,
        display_message,
    } = request;
    let command = access.into_observed_command(persistence.inner())?;
    if normalize_action_kind(&command.action.kind) != "file_read" {
        return Err(approved_chat_file_error(
            "Approved chat file preparation only supports viewing a file.",
        ));
    }
    let requested_path = command
        .action
        .path
        .as_deref()
        .ok_or_else(|| approved_chat_file_error("Choose a file to continue."))?;
    let (canonical_path, metadata, target_is_directory) =
        inspect_approved_chat_read_target(&command.action, requested_path)?;
    let display_name = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("selected file")
        .to_string();
    let target_identity_hash = sha256_hex(
        format!(
            "{}:{}:{}",
            canonical_path.display(),
            metadata.dev(),
            metadata.ino()
        )
        .as_bytes(),
    );
    let session_id =
        required_approved_chat_file_binding("a session", command.session_id.as_deref())?;
    let issued_turn_id = required_approved_chat_file_binding("a turn", command.turn_id.as_deref())?;
    let root_turn_id =
        required_approved_chat_file_binding("a root turn", command.root_turn_id.as_deref())?;
    let agent_id = required_approved_chat_file_binding("an agent", command.agent_id.as_deref())?;
    let display_message = display_message.trim().to_string();
    if display_message.is_empty() {
        return Err(approved_chat_file_error(
            "The original request is unavailable. Try again.",
        ));
    }

    let receipt_identity = identity.inner().clone();
    let receipt_context = ApprovedReceiptContext {
        canonical_path,
        metadata,
        target_is_directory,
        display_name,
        target_identity_hash,
        session_id,
        issued_turn_id,
        root_turn_id,
        agent_id,
        display_message,
    };
    let response = execute_native_file_access_command(
        command,
        persistence,
        identity,
        approvals,
        scope_trust,
        leases,
        app,
    )
    .await?;
    if !response.verified || response.status.as_str() != "completed" {
        return Err(approved_chat_file_error(
            "The selected file could not be viewed safely.",
        ));
    }
    let content = response.message.trim().to_string();
    if content.is_empty() || content.len() > MAX_APPROVED_CHAT_FILE_CONTEXT_BYTES {
        return Err(approved_chat_file_error(
            "The selected file could not be prepared for this chat.",
        ));
    }

    build_approved_receipt(receipt_context, content, &receipt_identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn identity() -> SovereignIdentity {
        SovereignIdentity::initialize_with_session_passphrase(
            "OOMU native file authority boundary test identity",
        )
        .expect("test identity initializes")
    }

    fn test_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is available")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "oomu-native-file-authority-{}-{suffix}-{}",
            std::process::id(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("test root is created");
        root
    }

    fn request(path: &Path) -> ExecuteCommandRequest {
        ExecuteCommandRequest {
            action: RequestedAction {
                kind: "file_read".to_string(),
                principal: Some("agent-a".to_string()),
                path: Some(path.display().to_string()),
                content: None,
            },
            logical_certificate: None,
            session_id: Some("session-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            generation_token: Some("generation-a".to_string()),
            agent_id: Some("agent-a".to_string()),
            provider_id: Some("provider-a".to_string()),
            model_id: Some("model-a".to_string()),
            parent_turn_id: None,
            root_turn_id: Some("turn-a".to_string()),
            turn_kind: Some("root".to_string()),
            project_id: None,
            task_run_id: None,
        }
    }

    #[test]
    fn renderer_request_rejects_certificate_and_signature_fields() {
        let value = serde_json::json!({
            "action": { "kind": "file_read", "path": "/tmp/example" },
            "sessionId": "session-a",
            "turnId": "turn-a",
            "generationToken": "generation-a",
            "signature": "renderer-controlled"
        });
        assert!(serde_json::from_value::<NativeDirectFileAccessRequest>(value).is_err());
    }

    #[test]
    fn native_authority_expands_home_shorthand_before_canonicalization() {
        let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let Ok(canonical_home) = fs::canonicalize(home) else {
            return;
        };

        let (target, root) = canonical_target_and_root("~").expect("home shorthand resolves");
        let (child_target, child_root) =
            canonical_target_and_root("~/.").expect("home child shorthand resolves");

        assert_eq!(target, canonical_home);
        assert_eq!(root, canonical_home);
        assert_eq!(child_target, canonical_home);
        assert_eq!(child_root, canonical_home);
        assert!(canonical_target_and_root("~/Downloads/../Documents").is_err());
    }

    #[test]
    fn authority_binds_path_action_session_turn_generation_and_agent() {
        let root = test_root();
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        fs::write(&first, b"first").expect("first fixture is written");
        fs::write(&second, b"second").expect("second fixture is written");
        let identity = identity();
        let mut issued_request = request(&first);
        let envelope = issue(&mut issued_request, &identity).expect("authority is issued");

        let mut mismatches = Vec::new();
        let mut path_mismatch = request(&second);
        path_mismatch.action.path = Some(second.display().to_string());
        mismatches.push(path_mismatch);
        let mut action_mismatch = request(&first);
        action_mismatch.action.kind = "file_list".to_string();
        mismatches.push(action_mismatch);
        let mut session_mismatch = request(&first);
        session_mismatch.session_id = Some("session-b".to_string());
        mismatches.push(session_mismatch);
        let mut turn_mismatch = request(&first);
        turn_mismatch.turn_id = Some("turn-b".to_string());
        mismatches.push(turn_mismatch);
        let mut generation_mismatch = request(&first);
        generation_mismatch.generation_token = Some("generation-b".to_string());
        mismatches.push(generation_mismatch);
        let mut agent_mismatch = request(&first);
        agent_mismatch.agent_id = Some("agent-b".to_string());
        mismatches.push(agent_mismatch);

        for mismatch in mismatches {
            let error = consume(&mismatch, &envelope, &identity)
                .expect_err("each authority mismatch must fail before I/O");
            assert_eq!(error.code, "security_boundary_violation");
        }
        fs::remove_dir_all(root).expect("test root is removed");
    }

    #[test]
    fn authority_expires_and_is_consumed_only_once() {
        let root = test_root();
        let file = root.join("fixture.txt");
        fs::write(&file, b"fixture").expect("fixture is written");
        let identity = identity();
        let mut issued_request = request(&file);
        let envelope = issue(&mut issued_request, &identity).expect("authority is issued");

        consume(&issued_request, &envelope, &identity).expect("first consumption succeeds");
        assert!(consume(&issued_request, &envelope, &identity).is_err());

        let mut expired_claim = envelope.claim.clone();
        expired_claim.issued_at_ms = 1;
        expired_claim.expires_at_ms = 2;
        let expired = identity
            .sign_native_file_authority(expired_claim)
            .expect("expired test claim is genuinely signed");
        assert!(consume(&issued_request, &expired, &identity).is_err());
        fs::remove_dir_all(root).expect("test root is removed");
    }

    #[test]
    fn directory_read_authority_binds_the_effective_file_list_action() {
        let root = test_root();
        let identity = identity();
        let mut issued_request = request(&root);
        let envelope = issue_native_file_authority(&mut issued_request, &identity)
            .expect("directory authority is issued");

        assert_eq!(issued_request.action.kind, "file_list");
        assert_eq!(envelope.claim.action_kind, "file_list");
        consume(&issued_request, &envelope, &identity)
            .expect("the normalized directory authority is consumable");
        fs::remove_dir_all(root).expect("test root is removed");
    }
}
