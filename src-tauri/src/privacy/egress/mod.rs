use crate::foundation::{clock::unix_time_ms_i64, digest::sha256_hex};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

const RECEIPT_TTL_MS: i64 = 5 * 60 * 1_000;
const REDACTED_EXCERPT_MAX_BYTES: usize = 16 * 1_024;

pub trait PrivateEgressAttachment {
    fn name(&self) -> &str;
    fn data_base64(&self) -> Option<&str>;
    fn text(&self) -> Option<&str>;
    fn text_mut(&mut self) -> Option<&mut String>;
    fn set_name(&mut self, name: String);
    fn set_byte_count(&mut self, byte_count: usize);
}

pub trait PrivateEgressMessage {
    type Attachment: PrivateEgressAttachment;

    fn attachments(&self) -> &[Self::Attachment];
    fn attachments_mut(&mut self) -> &mut [Self::Attachment];
}

pub trait PrivateEgressAuthority {
    fn sign_private_egress(&self, payload: &str) -> Result<String, String>;
    fn verify_private_egress(&self, payload: &str, signature_json: &str) -> Result<(), String>;
}

pub trait PrivateEgressStore {
    fn authenticate_native_public_search(
        &self,
        claim: &NativePublicSearchClaim,
    ) -> Result<bool, String>;

    fn store_private_egress_receipt(
        &self,
        receipt: &PrivateEgressReceiptPayload,
        signature_json: &str,
    ) -> Result<(), String>;

    fn consume_private_egress_receipt(
        &self,
        receipt: &PrivateEgressReceiptPayload,
        consumed_at_ms: i64,
    ) -> Result<bool, String>;

    fn find_private_egress_challenge(
        &self,
        binding: &PrivateEgressChallengeBinding,
    ) -> Result<Option<StoredPrivateEgressChallenge>, String>;

    fn store_private_egress_challenge(
        &self,
        challenge: &PrivateEgressChallengePayload,
    ) -> Result<(), String>;

    fn consume_private_egress_challenge(
        &self,
        challenge: &PrivateEgressChallengePayload,
        consumed_at_ms: i64,
    ) -> Result<bool, String>;

    fn has_consumed_private_egress_turn_approval(
        &self,
        _binding: &PrivateEgressChallengeBinding,
        _now_ms: i64,
    ) -> Result<bool, String> {
        Ok(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePublicSearchClaim {
    pub session_id: String,
    pub receipt_digest: String,
    pub invocation_index: usize,
    pub query_digest: String,
    pub context_digest: String,
    pub engine: String,
    pub result_count: usize,
    pub source_urls: Vec<String>,
    pub accessed_at_utc: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivateSourceKind {
    Mail,
    Calendar,
    Contacts,
    Photos,
    Files,
    Notes,
    Reminders,
    Messages,
    Connector,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateDataProvenance {
    pub source_kind: PrivateSourceKind,
    pub source_label: String,
    pub source_digest: String,
    pub sensitivity: String,
    pub local_turn_id: String,
    pub acquired_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateEgressReceiptPayload {
    pub receipt_id: String,
    pub source_digest: String,
    pub destination_provider_id: String,
    pub destination_model_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub allowed_representation: String,
    pub representation_digest: String,
    pub expires_at_ms: i64,
    pub dispatch_id: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct PrivateEgressChallengeBinding {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub destination_provider_id: String,
    pub destination_model_id: String,
    pub source_digest: String,
    pub allowed_representation: String,
    pub representation_digest: String,
}

#[derive(Debug, Clone)]
pub struct PrivateEgressChallengePayload {
    pub challenge_id: String,
    pub binding: PrivateEgressChallengeBinding,
    pub source_names: Vec<String>,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct StoredPrivateEgressChallenge {
    pub payload: PrivateEgressChallengePayload,
    pub decision: String,
}

#[derive(Debug, Clone)]
pub struct PrivateEgressError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone)]
pub struct PrivateEgressPermit {
    payload: Option<PrivateEgressReceiptPayload>,
    signature_json: Option<String>,
    session_id: String,
    turn_id: String,
    consumed: Arc<AtomicBool>,
    authenticated_public_searches: HashSet<String>,
}

impl PrivateEgressPermit {
    pub fn validate_and_consume<
        M: PrivateEgressMessage,
        S: PrivateEgressStore,
        A: PrivateEgressAuthority,
    >(
        &self,
        provider_id: &str,
        model_id: &str,
        session_id: &str,
        turn_id: &str,
        messages: &[M],
        persistence: &S,
        identity: &A,
    ) -> Result<(), PrivateEgressError> {
        if session_id != self.session_id || turn_id != self.turn_id {
            return Err(error(
                "private_egress_turn_changed",
                "The cloud request changed turns before dispatch. Nothing was sent.",
            ));
        }
        let material =
            private_material(messages, &self.turn_id, &self.authenticated_public_searches)?;
        let Some(payload) = self.payload.as_ref() else {
            if material.sources.is_empty() {
                return Ok(());
            }
            return Err(error(
                "private_egress_payload_changed",
                "The public evidence changed before it could be sent. Nothing was sent.",
            ));
        };
        let Some(signature_json) = self.signature_json.as_deref() else {
            return Err(error(
                "private_egress_signature_invalid",
                "OOMU could not verify this send approval. Nothing was sent.",
            ));
        };
        if provider_id != payload.destination_provider_id
            || model_id != payload.destination_model_id
        {
            return Err(error(
                "private_egress_destination_changed",
                "The selected cloud destination changed. Nothing was sent.",
            ));
        }
        if material.representation_digest != payload.representation_digest
            || material.representation != payload.allowed_representation
        {
            return Err(error(
                "private_egress_payload_changed",
                "The private content changed after approval. Nothing was sent.",
            ));
        }
        let canonical = canonical_payload(payload)?;
        identity
            .verify_private_egress(&canonical, signature_json)
            .map_err(|_| {
                error(
                    "private_egress_signature_invalid",
                    "OOMU could not verify this send approval. Nothing was sent.",
                )
            })?;
        if unix_time_ms_i64() > payload.expires_at_ms {
            return Err(error(
                "private_egress_receipt_expired",
                "This send approval expired. Nothing was sent.",
            ));
        }
        if self.consumed.load(Ordering::Acquire) {
            // Backend-owned retries may reuse the in-memory permit only for the
            // exact provider, model, turn, representation, and payload digest.
            return Ok(());
        }
        let consumed = persistence
            .consume_private_egress_receipt(payload, unix_time_ms_i64())
            .map_err(|_| {
                error(
                    "private_egress_receipt_consume_failed",
                    "OOMU could not verify this send approval. Nothing was sent.",
                )
            })?;
        if !consumed {
            return Err(error(
                "private_egress_receipt_unavailable",
                "This send approval is no longer available. Nothing was sent.",
            ));
        }
        self.consumed.store(true, Ordering::Release);
        Ok(())
    }
}

pub fn contains_private_data<M: PrivateEgressMessage>(messages: &[M]) -> bool {
    messages
        .iter()
        .flat_map(PrivateEgressMessage::attachments)
        .next()
        .is_some()
}

pub fn prepare_cloud_egress<
    M: PrivateEgressMessage,
    S: PrivateEgressStore,
    A: PrivateEgressAuthority,
>(
    messages: &mut [M],
    provider_id: &str,
    model_id: &str,
    session_id: &str,
    turn_id: &str,
    generation_token: &str,
    persistence: &S,
    identity: &A,
) -> Result<Option<PrivateEgressPermit>, PrivateEgressError> {
    let authenticated_public_searches =
        authenticated_public_searches(messages, session_id, persistence)?;
    let original_material = private_material(messages, turn_id, &authenticated_public_searches)?;
    minimize_private_attachments(messages, turn_id, &authenticated_public_searches)?;
    let material = private_material(messages, turn_id, &authenticated_public_searches)?;
    if material.sources.is_empty() {
        return Ok(
            (!authenticated_public_searches.is_empty()).then(|| PrivateEgressPermit {
                payload: None,
                signature_json: None,
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                consumed: Arc::new(AtomicBool::new(false)),
                authenticated_public_searches,
            }),
        );
    }
    let now = unix_time_ms_i64();
    let binding = PrivateEgressChallengeBinding {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        generation_token: generation_token.to_string(),
        destination_provider_id: provider_id.to_string(),
        destination_model_id: model_id.to_string(),
        source_digest: original_material.source_digest.clone(),
        allowed_representation: material.representation.clone(),
        representation_digest: material.representation_digest.clone(),
    };
    let challenge = persistence
        .find_private_egress_challenge(&binding)
        .map_err(|_| confirmation_error("private_egress_confirmation_unavailable"))?;
    let mut consume_exact_challenge = false;
    let turn_approval_reused = match challenge.as_ref() {
        Some(challenge) => {
            if now > challenge.payload.expires_at_ms {
                return Err(confirmation_error("private_egress_confirmation_expired"));
            }
            match challenge.decision.as_str() {
                "approved" => {
                    consume_exact_challenge = true;
                    false
                }
                "consumed" => true,
                "denied" => {
                    return Err(error(
                        "private_egress_user_denied",
                        "Your private information stayed on this Mac.",
                    ));
                }
                _ => return Err(confirmation_error("private_egress_confirmation_required")),
            }
        }
        None => persistence
            .has_consumed_private_egress_turn_approval(&binding, now)
            .map_err(|_| confirmation_error("private_egress_confirmation_unavailable"))?,
    };
    if challenge.is_none() && !turn_approval_reused {
        let challenge = PrivateEgressChallengePayload {
            challenge_id: format!("egress_confirm_{}", random_token()),
            binding,
            source_names: original_material.source_names.clone(),
            expires_at_ms: now.saturating_add(RECEIPT_TTL_MS),
            created_at_ms: now,
        };
        persistence
            .store_private_egress_challenge(&challenge)
            .map_err(|_| confirmation_error("private_egress_confirmation_unavailable"))?;
        return Err(confirmation_error("private_egress_confirmation_required"));
    }
    let nonce = random_token();
    let receipt_id = format!(
        "egress_{}",
        sha256_hex(format!("{session_id}:{turn_id}:{nonce}:{now}").as_bytes())
    );
    let dispatch_id = format!(
        "dispatch_{}",
        sha256_hex(format!("{receipt_id}:{provider_id}:{model_id}").as_bytes())
    );
    let payload = PrivateEgressReceiptPayload {
        receipt_id: receipt_id.clone(),
        source_digest: original_material.source_digest,
        destination_provider_id: provider_id.to_string(),
        destination_model_id: model_id.to_string(),
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        allowed_representation: material.representation,
        representation_digest: material.representation_digest,
        expires_at_ms: now.saturating_add(RECEIPT_TTL_MS),
        dispatch_id,
        created_at_ms: now,
    };
    let canonical = canonical_payload(&payload)?;
    let signature_json = identity.sign_private_egress(&canonical).map_err(|_| {
        error(
            "private_egress_signing_unavailable",
            "OOMU could not secure this send approval. Nothing was sent.",
        )
    })?;
    persistence
        .store_private_egress_receipt(&payload, &signature_json)
        .map_err(|_| {
            error(
                "private_egress_receipt_store_failed",
                "OOMU could not secure this send approval. Nothing was sent.",
            )
        })?;
    if consume_exact_challenge {
        let Some(challenge) = challenge.as_ref() else {
            return Err(confirmation_error("private_egress_confirmation_invalid"));
        };
        if !persistence
            .consume_private_egress_challenge(&challenge.payload, now)
            .map_err(|_| confirmation_error("private_egress_confirmation_unavailable"))?
        {
            return Err(confirmation_error("private_egress_confirmation_invalid"));
        }
    }
    Ok(Some(PrivateEgressPermit {
        session_id: payload.session_id.clone(),
        turn_id: payload.turn_id.clone(),
        payload: Some(payload),
        signature_json: Some(signature_json),
        consumed: Arc::new(AtomicBool::new(false)),
        authenticated_public_searches,
    }))
}

fn confirmation_error(code: &'static str) -> PrivateEgressError {
    error(
        code,
        "Choose whether to send this private information to the selected cloud model.",
    )
}

struct PrivateMaterial {
    sources: Vec<PrivateDataProvenance>,
    source_names: Vec<String>,
    source_digest: String,
    representation_digest: String,
    representation: String,
}

fn private_material<M: PrivateEgressMessage>(
    messages: &[M],
    turn_id: &str,
    authenticated_public_searches: &HashSet<String>,
) -> Result<PrivateMaterial, PrivateEgressError> {
    let mut sources = Vec::new();
    let mut source_names = Vec::new();
    let mut representations = Vec::new();
    for attachment in messages
        .iter()
        .flat_map(PrivateEgressMessage::attachments)
        .filter(|attachment| {
            !is_authenticated_public_search(*attachment, authenticated_public_searches)
        })
    {
        let bytes = attachment_bytes(attachment)?;
        let exact_digest = sha256_hex(&bytes);
        let provenance = PrivateDataProvenance {
            source_kind: source_kind(attachment),
            source_label: source_label(attachment),
            source_digest: exact_digest.clone(),
            sensitivity: "private".to_string(),
            local_turn_id: turn_id.to_string(),
            acquired_at_ms: unix_time_ms_i64(),
        };
        if provenance.sensitivity != "private" && provenance.sensitivity != "restricted" {
            return Err(error(
                "private_provenance_invalid",
                "OOMU could not verify the private source. Nothing was sent.",
            ));
        }
        representations.push(format!("{}:{}", attachment.name(), exact_digest));
        source_names.push(attachment.name().to_string());
        sources.push(provenance);
    }
    let mut source_entries = sources
        .iter()
        .map(|source| format!("{:?}:{}", source.source_kind, source.source_digest))
        .collect::<Vec<_>>();
    source_entries.sort();
    representations.sort();
    source_names.sort();
    source_names.dedup();
    let representation = if messages
        .iter()
        .flat_map(PrivateEgressMessage::attachments)
        .any(|attachment| attachment.name().ends_with(".redacted.txt"))
    {
        "redacted_excerpt"
    } else if !sources.is_empty()
        && messages
            .iter()
            .flat_map(PrivateEgressMessage::attachments)
            .filter(|attachment| {
                !is_authenticated_public_search(*attachment, authenticated_public_searches)
            })
            .all(|attachment| attachment.name().ends_with(".local-summary.txt"))
    {
        "local_summary"
    } else {
        "full_result"
    }
    .to_string();
    Ok(PrivateMaterial {
        sources,
        source_names,
        source_digest: sha256_hex(source_entries.join("\n").as_bytes()),
        representation_digest: sha256_hex(representations.join("\n").as_bytes()),
        representation,
    })
}

fn minimize_private_attachments<M: PrivateEgressMessage>(
    messages: &mut [M],
    _turn_id: &str,
    authenticated_public_searches: &HashSet<String>,
) -> Result<(), PrivateEgressError> {
    for attachment in messages
        .iter_mut()
        .flat_map(PrivateEgressMessage::attachments_mut)
        .filter(|attachment| {
            !is_authenticated_public_search(&**attachment, authenticated_public_searches)
        })
    {
        let original_bytes = attachment_bytes(attachment)?;
        let _original_digest = sha256_hex(&original_bytes);
        let original_name = attachment.name().to_string();
        if let Some(text) = attachment.text_mut() {
            if text.len() > REDACTED_EXCERPT_MAX_BYTES {
                let end = text.floor_char_boundary(REDACTED_EXCERPT_MAX_BYTES);
                text.truncate(end);
                text.push_str("\n\n[Shortened on this Mac before sending]");
                let byte_count = text.len();
                attachment.set_byte_count(byte_count);
                attachment.set_name(format!("{original_name}.redacted.txt"));
            }
        }
    }
    Ok(())
}

fn authenticated_public_searches<M: PrivateEgressMessage, S: PrivateEgressStore>(
    messages: &[M],
    session_id: &str,
    persistence: &S,
) -> Result<HashSet<String>, PrivateEgressError> {
    let mut authenticated = HashSet::new();
    for attachment in messages.iter().flat_map(PrivateEgressMessage::attachments) {
        let Some(claim) = native_public_search_claim(attachment, session_id) else {
            continue;
        };
        if persistence
            .authenticate_native_public_search(&claim)
            .map_err(|_| {
                error(
                    "private_source_verification_unavailable",
                    "OOMU could not verify whether this source is public. Nothing was sent.",
                )
            })?
        {
            authenticated.insert(attachment_identity(attachment)?);
        }
    }
    Ok(authenticated)
}

fn is_authenticated_public_search<A: PrivateEgressAttachment>(
    attachment: &A,
    authenticated: &HashSet<String>,
) -> bool {
    attachment_identity(attachment)
        .ok()
        .is_some_and(|identity| authenticated.contains(&identity))
}

fn is_public_search_attachment_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized == "local_web_search.md"
        || normalized
            .strip_prefix("local_web_search_")
            .and_then(|suffix| suffix.strip_suffix(".md"))
            .is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn attachment_identity<A: PrivateEgressAttachment>(
    attachment: &A,
) -> Result<String, PrivateEgressError> {
    let bytes = attachment_bytes(attachment)?;
    Ok(sha256_hex(
        [attachment.name().as_bytes(), b"\0", bytes.as_slice()]
            .concat()
            .as_slice(),
    ))
}

fn native_public_search_claim<A: PrivateEgressAttachment>(
    attachment: &A,
    session_id: &str,
) -> Option<NativePublicSearchClaim> {
    if !is_public_search_attachment_name(attachment.name()) || attachment.data_base64().is_some() {
        return None;
    }
    let (query, engine, receipt_digest, invocation_index, result_count, context_json) =
        validated_public_search_envelope(attachment.text()?)?;
    let context: serde_json::Value = serde_json::from_str(context_json).ok()?;
    let accessed_at_utc = context.get("accessedAtUtc")?.as_str()?.to_string();
    let source_urls = crate::foundation::public_web_sources::from_context_json(context_json)
        .into_iter()
        .map(|source| source.url)
        .collect::<Vec<_>>();
    if source_urls.is_empty() {
        return None;
    }
    Some(NativePublicSearchClaim {
        session_id: session_id.to_string(),
        receipt_digest: receipt_digest.to_ascii_lowercase(),
        invocation_index,
        query_digest: sha256_hex(query.as_bytes()),
        context_digest: sha256_hex(context_json.as_bytes()),
        engine: engine.to_string(),
        result_count,
        source_urls,
        accessed_at_utc,
    })
}

fn validated_public_search_envelope(text: &str) -> Option<(&str, &str, &str, usize, usize, &str)> {
    let (header_text, context_json) = text.split_once("\n\n")?;
    let headers = header_text.lines().collect::<Vec<_>>();
    if headers.len() != 7
        || headers[0] != "Local Web Search Context"
        || headers[6] != "Isolation: keyless public search plus sanitized DOM streaming; no API key; no persistent cookies; no proxy environment; no visible browser panel."
    {
        return None;
    }
    let query = exact_public_search_header(&headers, 1, "Query: ")?;
    let canonical_query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query != canonical_query
        || query.chars().count() > 500
        || query.chars().any(char::is_control)
    {
        return None;
    }
    let engine = exact_public_search_header(&headers, 2, "Engine: ")?;
    if engine.chars().count() > 128 || engine.chars().any(char::is_control) {
        return None;
    }
    let receipt_digest = exact_public_search_header(&headers, 3, "Native-Receipt: ")?;
    if receipt_digest.len() != 64
        || !receipt_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let invocation_index = exact_public_search_header(&headers, 4, "Invocation-Index: ")?
        .parse::<usize>()
        .ok()?;
    let result_count = exact_public_search_header(&headers, 5, "Result-Count: ")?
        .parse::<usize>()
        .ok()?;
    if invocation_index == 0 || result_count == 0 {
        return None;
    }
    if context_json.trim() != context_json || !context_json.starts_with('{') {
        return None;
    }
    Some((
        query,
        engine,
        receipt_digest,
        invocation_index,
        result_count,
        context_json,
    ))
}

fn exact_public_search_header<'a>(
    headers: &[&'a str],
    index: usize,
    prefix: &str,
) -> Option<&'a str> {
    headers
        .get(index)?
        .strip_prefix(prefix)
        .filter(|value| !value.is_empty() && value.trim() == *value)
}

fn attachment_bytes<A: PrivateEgressAttachment>(
    attachment: &A,
) -> Result<Vec<u8>, PrivateEgressError> {
    if let Some(data) = attachment.data_base64() {
        return BASE64_STANDARD.decode(data).map_err(|_| {
            error(
                "private_source_invalid",
                "OOMU could not verify the private source. Nothing was sent.",
            )
        });
    }
    Ok(attachment.text().unwrap_or_default().as_bytes().to_vec())
}

fn source_kind<A: PrivateEgressAttachment>(attachment: &A) -> PrivateSourceKind {
    let name = attachment.name().to_ascii_lowercase();
    if name.contains("mail") {
        PrivateSourceKind::Mail
    } else if name.contains("calendar") {
        PrivateSourceKind::Calendar
    } else if name.contains("contact") {
        PrivateSourceKind::Contacts
    } else if name.contains("photo") {
        PrivateSourceKind::Photos
    } else if name.contains("note") {
        PrivateSourceKind::Notes
    } else if name.contains("reminder") {
        PrivateSourceKind::Reminders
    } else if name.contains("message") {
        PrivateSourceKind::Messages
    } else if name.starts_with("connector_") {
        PrivateSourceKind::Connector
    } else {
        PrivateSourceKind::Files
    }
}

fn source_label<A: PrivateEgressAttachment>(attachment: &A) -> String {
    match source_kind(attachment) {
        PrivateSourceKind::Mail => "Mail on this Mac",
        PrivateSourceKind::Calendar => "Calendar on this Mac",
        PrivateSourceKind::Contacts => "Contacts on this Mac",
        PrivateSourceKind::Photos => "Photos on this Mac",
        PrivateSourceKind::Files => "a local file",
        PrivateSourceKind::Notes => "Notes on this Mac",
        PrivateSourceKind::Reminders => "Reminders on this Mac",
        PrivateSourceKind::Messages => "Messages on this Mac",
        PrivateSourceKind::Connector => "a connected private service",
    }
    .to_string()
}

fn canonical_payload(payload: &PrivateEgressReceiptPayload) -> Result<String, PrivateEgressError> {
    serde_json::to_string(payload).map_err(|_| {
        error(
            "private_egress_receipt_invalid",
            "OOMU could not secure this send approval. Nothing was sent.",
        )
    })
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn error(code: &'static str, message: &str) -> PrivateEgressError {
    PrivateEgressError {
        code,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        db::PersistenceEngine,
        inference::{ChatAttachment, InferenceMessage},
        sovereign_identity::SovereignIdentity,
    };
    use super::*;
    use rusqlite::params;

    fn attachment(name: &str, text: &str) -> ChatAttachment {
        ChatAttachment {
            name: name.to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: text.len(),
            data_base64: None,
            text: Some(text.to_string()),
            approved_file_receipt: None,
        }
    }

    struct PublicSearchStore;

    impl PrivateEgressStore for PublicSearchStore {
        fn authenticate_native_public_search(
            &self,
            _claim: &NativePublicSearchClaim,
        ) -> Result<bool, String> {
            Ok(true)
        }

        fn store_private_egress_receipt(
            &self,
            _receipt: &PrivateEgressReceiptPayload,
            _signature_json: &str,
        ) -> Result<(), String> {
            Err("private receipt must not be created for public evidence".to_string())
        }

        fn consume_private_egress_receipt(
            &self,
            _receipt: &PrivateEgressReceiptPayload,
            _consumed_at_ms: i64,
        ) -> Result<bool, String> {
            Err("private receipt must not be consumed for public evidence".to_string())
        }

        fn find_private_egress_challenge(
            &self,
            _binding: &PrivateEgressChallengeBinding,
        ) -> Result<Option<StoredPrivateEgressChallenge>, String> {
            Err("private confirmation must not be requested for public evidence".to_string())
        }

        fn store_private_egress_challenge(
            &self,
            _challenge: &PrivateEgressChallengePayload,
        ) -> Result<(), String> {
            Err("private confirmation must not be stored for public evidence".to_string())
        }

        fn consume_private_egress_challenge(
            &self,
            _challenge: &PrivateEgressChallengePayload,
            _consumed_at_ms: i64,
        ) -> Result<bool, String> {
            Err("private confirmation must not be consumed for public evidence".to_string())
        }
    }

    fn receipt_backed_public_search_attachment() -> ChatAttachment {
        let context = serde_json::json!({
            "accessedAtUtc": "2026-08-01T20:30:00.000Z",
            "pages": [{"url": "https://www.wiley.com/example"}],
        });
        attachment(
            "local_web_search.md",
            &format!(
                concat!(
                    "Local Web Search Context\n",
                    "Query: Writing AI Prompts for Dummies latest edition\n",
                    "Engine: duckduckgo_lite_static\n",
                    "Native-Receipt: {}\n",
                    "Invocation-Index: 1\n",
                    "Result-Count: 1\n",
                    "Isolation: keyless public search plus sanitized DOM streaming; no API key; no persistent cookies; no proxy environment; no visible browser panel.\n\n",
                    "{}"
                ),
                "a".repeat(64),
                context
            ),
        )
    }

    fn persisted_test_permit(
        messages: &mut [InferenceMessage],
        persistence: &PersistenceEngine,
        identity: &SovereignIdentity,
    ) -> PrivateEgressPermit {
        let authenticated_public_searches = HashSet::new();
        minimize_private_attachments(messages, "turn-1", &authenticated_public_searches).unwrap();
        let material =
            private_material(messages, "turn-1", &authenticated_public_searches).unwrap();
        let now = unix_time_ms_i64();
        let payload = PrivateEgressReceiptPayload {
            receipt_id: format!("egress-test-{now}"),
            source_digest: material.source_digest,
            destination_provider_id: "google-gemini".to_string(),
            destination_model_id: "gemini-test".to_string(),
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            allowed_representation: material.representation,
            representation_digest: material.representation_digest,
            expires_at_ms: now + RECEIPT_TTL_MS,
            dispatch_id: format!("dispatch-test-{now}"),
            created_at_ms: now,
        };
        let canonical = canonical_payload(&payload).unwrap();
        let signature_json = identity.sign_private_egress(&canonical).unwrap();
        persistence
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO private_data_egress_receipts (
                receipt_id, source_digest, destination_provider_id, destination_model_id,
                session_id, turn_id, allowed_representation, representation_digest,
                expires_at_ms, consumed_at_ms, signature_json, dispatch_id, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)",
                params![
                    payload.receipt_id,
                    payload.source_digest,
                    payload.destination_provider_id,
                    payload.destination_model_id,
                    payload.session_id,
                    payload.turn_id,
                    payload.allowed_representation,
                    payload.representation_digest,
                    payload.expires_at_ms,
                    signature_json,
                    payload.dispatch_id,
                    payload.created_at_ms,
                ],
            )
            .unwrap();
        PrivateEgressPermit {
            session_id: payload.session_id.clone(),
            turn_id: payload.turn_id.clone(),
            payload: Some(payload),
            signature_json: Some(signature_json),
            consumed: Arc::new(AtomicBool::new(false)),
            authenticated_public_searches,
        }
    }

    #[test]
    fn private_egress_local_sources_are_classified_and_minimized() {
        let mut messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "summarize".to_string(),
            attachments: vec![attachment(
                "local_mail.json",
                &"x".repeat(REDACTED_EXCERPT_MAX_BYTES + 10),
            )],
        }];
        let authenticated_public_searches = HashSet::new();
        minimize_private_attachments(&mut messages, "turn-1", &authenticated_public_searches)
            .unwrap();
        let material =
            private_material(&messages, "turn-1", &authenticated_public_searches).unwrap();
        assert_eq!(material.representation, "redacted_excerpt");
        assert_eq!(material.sources[0].source_kind, PrivateSourceKind::Mail);
        assert!(messages[0].attachments[0]
            .text
            .as_ref()
            .unwrap()
            .contains("Shortened on this Mac"));
    }

    #[test]
    fn private_file_named_local_web_search_never_bypasses_egress_review() {
        let messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "summarize my payroll".to_string(),
            attachments: vec![attachment("local_web_search.md", "private payroll records")],
        }];
        assert!(contains_private_data(&messages));

        let numbered = vec![InferenceMessage {
            role: "user".to_string(),
            content: "continue search".to_string(),
            attachments: vec![attachment("local_web_search_2.md", "public result")],
        }];
        assert!(contains_private_data(&numbered));
    }

    #[test]
    fn authenticated_public_search_gets_a_bound_dispatch_permit_without_private_review() {
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "Answer from verified public evidence".to_string(),
            attachments: vec![receipt_backed_public_search_attachment()],
        }];
        assert!(contains_private_data(&messages));

        let permit = prepare_cloud_egress(
            &mut messages,
            "google-gemini",
            "gemini-test",
            "session-search",
            "turn-search-continuation",
            "generation-search-continuation",
            &PublicSearchStore,
            &identity,
        )
        .expect("verified public evidence needs no private confirmation")
        .expect("verified public evidence receives a dispatch-bound assessment");
        permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-search",
                "turn-search-continuation",
                &messages,
                &PublicSearchStore,
                &identity,
            )
            .expect("unchanged authenticated public evidence may reach the cloud model");

        let mut changed = messages.clone();
        changed[0].attachments[0]
            .text
            .as_mut()
            .unwrap()
            .push_str("\nprivate addition");
        assert_eq!(
            permit
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-search",
                    "turn-search-continuation",
                    &changed,
                    &PublicSearchStore,
                    &identity,
                )
                .unwrap_err()
                .code,
            "private_egress_payload_changed"
        );

        let mut mixed = messages.clone();
        mixed[0]
            .attachments
            .push(attachment("local_mail.json", "private mailbox content"));
        assert_eq!(
            permit
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-search",
                    "turn-search-continuation",
                    &mixed,
                    &PublicSearchStore,
                    &identity,
                )
                .unwrap_err()
                .code,
            "private_egress_payload_changed"
        );
        assert_eq!(
            permit
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-search",
                    "another-turn",
                    &messages,
                    &PublicSearchStore,
                    &identity,
                )
                .unwrap_err()
                .code,
            "private_egress_turn_changed"
        );
    }

    #[test]
    fn public_search_envelope_rejects_appended_or_pre_json_payloads() {
        let exact = receipt_backed_public_search_attachment();
        assert!(native_public_search_claim(&exact, "session-search").is_some());
        let mut numbered = exact.clone();
        numbered.name = "local_web_search_2.md".to_string();
        assert!(native_public_search_claim(&numbered, "session-search").is_some());

        let mut appended = exact.clone();
        appended
            .text
            .as_mut()
            .unwrap()
            .push_str("\nprivate appended payload");
        assert!(native_public_search_claim(&appended, "session-search").is_none());

        let mut pre_json = exact.clone();
        let text = pre_json.text.as_mut().unwrap();
        *text = text.replacen("\n\n{", "\n\nprivate pre-json payload\n{", 1);
        assert!(native_public_search_claim(&pre_json, "session-search").is_none());

        let mut altered_envelope = exact;
        let text = altered_envelope.text.as_mut().unwrap();
        *text = text.replacen(
            "Isolation: keyless public search plus sanitized DOM streaming;",
            "Isolation: injected private context;",
            1,
        );
        assert!(native_public_search_claim(&altered_envelope, "session-search").is_none());
    }

    #[test]
    fn private_egress_classifies_every_private_source_family() {
        for (name, expected) in [
            ("local_mail.json", PrivateSourceKind::Mail),
            ("local_calendar.json", PrivateSourceKind::Calendar),
            ("local_contacts.json", PrivateSourceKind::Contacts),
            ("local_photos.json", PrivateSourceKind::Photos),
            ("approved_report.pdf", PrivateSourceKind::Files),
            ("local_notes.json", PrivateSourceKind::Notes),
            ("local_reminders.json", PrivateSourceKind::Reminders),
            ("local_messages.json", PrivateSourceKind::Messages),
            ("connector_salesforce.json", PrivateSourceKind::Connector),
        ] {
            assert_eq!(source_kind(&attachment(name, "private")), expected);
        }
    }

    #[test]
    fn private_egress_failed_receipt_lookup_does_not_create_a_retry_bypass() {
        let root =
            std::env::temp_dir().join(format!("oomu-private-egress-atomic-{}", unix_time_ms_i64()));
        let valid = PersistenceEngine::initialize_at(root.join("valid.sqlite")).unwrap();
        let missing = PersistenceEngine::initialize_at(root.join("missing.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "summarize".to_string(),
            attachments: vec![attachment("local_mail.json", "private")],
        }];
        let permit = persisted_test_permit(&mut messages, &valid, &identity);

        assert_eq!(
            permit
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-1",
                    "turn-1",
                    &messages,
                    &missing,
                    &identity,
                )
                .unwrap_err()
                .code,
            "private_egress_receipt_unavailable"
        );
        permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-1",
                "turn-1",
                &messages,
                &valid,
                &identity,
            )
            .unwrap();
        let consumed_at_ms: Option<i64> = valid
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT consumed_at_ms FROM private_data_egress_receipts WHERE receipt_id = ?1",
                params![permit.payload.as_ref().unwrap().receipt_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed_at_ms.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn private_egress_receipt_replay_destination_and_digest_changes_fail_closed() {
        let root = std::env::temp_dir().join(format!("oomu-private-egress-{}", unix_time_ms_i64()));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "summarize".to_string(),
            attachments: vec![attachment("local_contacts.json", "Maya Allan")],
        }];
        let permit = persisted_test_permit(&mut messages, &persistence, &identity);
        let replay = PrivateEgressPermit {
            payload: permit.payload.clone(),
            signature_json: permit.signature_json.clone(),
            session_id: permit.session_id.clone(),
            turn_id: permit.turn_id.clone(),
            consumed: Arc::new(AtomicBool::new(false)),
            authenticated_public_searches: permit.authenticated_public_searches.clone(),
        };
        assert_eq!(
            permit
                .validate_and_consume(
                    "openai",
                    "gemini-test",
                    "session-1",
                    "turn-1",
                    &messages,
                    &persistence,
                    &identity,
                )
                .unwrap_err()
                .code,
            "private_egress_destination_changed"
        );
        permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-1",
                "turn-1",
                &messages,
                &persistence,
                &identity,
            )
            .unwrap();
        assert_eq!(
            replay
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-1",
                    "turn-1",
                    &messages,
                    &persistence,
                    &identity
                )
                .unwrap_err()
                .code,
            "private_egress_receipt_unavailable"
        );

        let mut changed = messages.clone();
        changed[0].attachments[0].text = Some("different private data".to_string());
        assert_eq!(
            permit
                .validate_and_consume(
                    "google-gemini",
                    "gemini-test",
                    "session-1",
                    "turn-1",
                    &changed,
                    &persistence,
                    &identity
                )
                .unwrap_err()
                .code,
            "private_egress_payload_changed"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn private_cloud_dispatch_waits_for_one_persisted_exact_confirmation() {
        let root = std::env::temp_dir().join(format!(
            "oomu-private-egress-confirmation-{}",
            unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "summarize".to_string(),
            attachments: vec![attachment("private-plan.md", "Release plan")],
        }];

        let first = match prepare_cloud_egress(
            &mut messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        ) {
            Ok(_) => panic!("private cloud egress must wait for an exact confirmation"),
            Err(error) => error,
        };
        assert_eq!(first.code, "private_egress_confirmation_required");
        let connection = persistence.open_connection().unwrap();
        let (challenge_id, decision, source_names): (String, String, String) = connection
            .query_row(
                "SELECT challenge_id,decision,source_names_json FROM private_egress_confirmation_challenges",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(decision, "pending");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&source_names).unwrap(),
            vec!["private-plan.md"]
        );
        connection
            .execute(
                "UPDATE private_egress_confirmation_challenges SET decision='approved',decided_at_ms=?2 WHERE challenge_id=?1",
                params![challenge_id, unix_time_ms_i64()],
            )
            .unwrap();
        drop(connection);

        let permit = prepare_cloud_egress(
            &mut messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        )
        .unwrap()
        .unwrap();
        permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-1",
                "turn-1",
                &messages,
                &persistence,
                &identity,
            )
            .unwrap();
        let connection = persistence.open_connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT decision FROM private_egress_confirmation_challenges WHERE challenge_id=?1",
                    params![challenge_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "consumed"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_reply_approval_covers_later_private_results_for_the_same_destination() {
        let root = std::env::temp_dir().join(format!(
            "oomu-private-egress-turn-approval-{}",
            unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut first_messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "compare".to_string(),
            attachments: vec![attachment("connector_read_file.json", "Supplier proposal")],
        }];

        let first_error = match prepare_cloud_egress(
            &mut first_messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        ) {
            Ok(_) => panic!("private material must require approval before it leaves the Mac"),
            Err(error) => error,
        };
        assert_eq!(first_error.code, "private_egress_confirmation_required");
        persistence
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE private_egress_confirmation_challenges
                 SET decision='approved',decided_at_ms=?1",
                params![unix_time_ms_i64()],
            )
            .unwrap();
        let first_permit = prepare_cloud_egress(
            &mut first_messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        )
        .unwrap()
        .unwrap();
        first_permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-1",
                "turn-1",
                &first_messages,
                &persistence,
                &identity,
            )
            .unwrap();

        let mut later_messages = vec![InferenceMessage {
            role: "user".to_string(),
            content: "compare".to_string(),
            attachments: vec![attachment(
                "connector_read_file.json",
                "Vendor requirements",
            )],
        }];
        let later_permit = prepare_cloud_egress(
            &mut later_messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        )
        .expect("the consumed approval should cover the rest of this reply")
        .unwrap();
        later_permit
            .validate_and_consume(
                "google-gemini",
                "gemini-test",
                "session-1",
                "turn-1",
                &later_messages,
                &persistence,
                &identity,
            )
            .unwrap();

        let connection = persistence.open_connection().unwrap();
        let challenge_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM private_egress_confirmation_challenges",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let receipt_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM private_data_egress_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(challenge_count, 1);
        assert_eq!(receipt_count, 2);
        drop(connection);

        let different_destination = match prepare_cloud_egress(
            &mut later_messages,
            "google-gemini",
            "gemini-other",
            "session-1",
            "turn-1",
            "generation-1",
            &persistence,
            &identity,
        ) {
            Ok(_) => panic!("approval must not carry to another destination model"),
            Err(error) => error,
        };
        assert_eq!(
            different_destination.code,
            "private_egress_confirmation_required"
        );

        let next_reply = match prepare_cloud_egress(
            &mut later_messages,
            "google-gemini",
            "gemini-test",
            "session-1",
            "turn-2",
            "generation-2",
            &persistence,
            &identity,
        ) {
            Ok(_) => panic!("approval must not carry to another reply"),
            Err(error) => error,
        };
        assert_eq!(next_reply.code, "private_egress_confirmation_required");
        let _ = std::fs::remove_dir_all(root);
    }
}
