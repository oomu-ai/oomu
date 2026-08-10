use crate::db::PersistenceEngine;
use crate::privacy::egress::{
    NativePublicSearchClaim, PrivateEgressChallengeBinding, PrivateEgressChallengePayload,
    PrivateEgressReceiptPayload, PrivateEgressStore, StoredPrivateEgressChallenge,
};
use rusqlite::{params, OptionalExtension};

pub fn authenticate_native_public_search(
    engine: &PersistenceEngine,
    claim: &NativePublicSearchClaim,
) -> Result<bool, String> {
    let messages = engine
        .select_chat_messages(&claim.session_id)
        .map_err(|error| error.to_string())?;
    Ok(messages.iter().any(|message| {
        let Some(metadata) = message
            .metadata_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        else {
            return false;
        };
        authenticated_search_metadata(&metadata, claim)
    }))
}

fn authenticated_search_metadata(
    metadata: &serde_json::Value,
    claim: &NativePublicSearchClaim,
) -> bool {
    let string = |key| metadata.get(key).and_then(serde_json::Value::as_str);
    let number = |key| metadata.get(key).and_then(serde_json::Value::as_u64);
    if string("checkpointKind") != Some("verified_sovereign_search")
        || string("eventKind") != Some("sovereign_search_receipt")
        || string("sessionId") != Some(claim.session_id.as_str())
        || string("searchReceiptDigest") != Some(claim.receipt_digest.as_str())
        || string("queryDigest") != Some(claim.query_digest.as_str())
        || string("contextDigest") != Some(claim.context_digest.as_str())
        || string("searchEngine") != Some(claim.engine.as_str())
        || string("accessedAtUtc") != Some(claim.accessed_at_utc.as_str())
        || number("searchInvocationIndex") != Some(claim.invocation_index as u64)
        || number("resultCount") != Some(claim.result_count as u64)
        || number("retrievedPageCount") != Some(claim.source_urls.len() as u64)
        || metadata.get("sourceUrls") != serde_json::to_value(&claim.source_urls).ok().as_ref()
    {
        return false;
    }
    let Some(turn_id) = string("turnId") else {
        return false;
    };
    let Some(generation_token) = string("generationToken") else {
        return false;
    };
    let Some(result_urls) = metadata.get("resultUrls") else {
        return false;
    };
    let receipt_payload = serde_json::json!({
        "sessionId": claim.session_id,
        "turnId": turn_id,
        "generationToken": generation_token,
        "queryDigest": claim.query_digest,
        "contextDigest": claim.context_digest,
        "engine": claim.engine,
        "resultCount": claim.result_count,
        "retrievedPageCount": claim.source_urls.len(),
        "sourceUrls": claim.source_urls,
        "resultUrls": result_urls,
        "accessedAtUtc": claim.accessed_at_utc,
        "invocationIndex": claim.invocation_index,
    });
    serde_json::to_string(&receipt_payload)
        .ok()
        .map(|payload| crate::foundation::digest::sha256_hex(payload.as_bytes()))
        .as_deref()
        == Some(claim.receipt_digest.as_str())
}

pub fn store_receipt(
    engine: &PersistenceEngine,
    receipt: &PrivateEgressReceiptPayload,
    signature_json: &str,
) -> Result<(), String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO private_data_egress_receipts (
                receipt_id, source_digest, destination_provider_id, destination_model_id,
                session_id, turn_id, allowed_representation, representation_digest,
                expires_at_ms, consumed_at_ms, signature_json, dispatch_id, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)",
            params![
                receipt.receipt_id,
                receipt.source_digest,
                receipt.destination_provider_id,
                receipt.destination_model_id,
                receipt.session_id,
                receipt.turn_id,
                receipt.allowed_representation,
                receipt.representation_digest,
                receipt.expires_at_ms,
                signature_json,
                receipt.dispatch_id,
                receipt.created_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn consume_receipt(
    engine: &PersistenceEngine,
    receipt: &PrivateEgressReceiptPayload,
    consumed_at_ms: i64,
) -> Result<bool, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE private_data_egress_receipts
             SET consumed_at_ms = ?2
             WHERE receipt_id = ?1 AND consumed_at_ms IS NULL AND expires_at_ms >= ?2
               AND destination_provider_id = ?3 AND destination_model_id = ?4
               AND source_digest = ?5 AND representation_digest = ?6
               AND session_id = ?7 AND turn_id = ?8",
            params![
                receipt.receipt_id,
                consumed_at_ms,
                receipt.destination_provider_id,
                receipt.destination_model_id,
                receipt.source_digest,
                receipt.representation_digest,
                receipt.session_id,
                receipt.turn_id,
            ],
        )
        .map(|count| count == 1)
        .map_err(|error| error.to_string())
}

pub fn find_challenge(
    engine: &PersistenceEngine,
    binding: &PrivateEgressChallengeBinding,
) -> Result<Option<StoredPrivateEgressChallenge>, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT challenge_id,source_names_json,expires_at_ms,created_at_ms,decision
             FROM private_egress_confirmation_challenges
             WHERE session_id=?1 AND turn_id=?2 AND generation_token=?3
               AND destination_provider_id=?4 AND destination_model_id=?5
               AND source_digest=?6 AND allowed_representation=?7
               AND representation_digest=?8",
            params![
                binding.session_id,
                binding.turn_id,
                binding.generation_token,
                binding.destination_provider_id,
                binding.destination_model_id,
                binding.source_digest,
                binding.allowed_representation,
                binding.representation_digest,
            ],
            |row| {
                let source_names_json: String = row.get(1)?;
                let source_names = serde_json::from_str(&source_names_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(StoredPrivateEgressChallenge {
                    payload: PrivateEgressChallengePayload {
                        challenge_id: row.get(0)?,
                        binding: binding.clone(),
                        source_names,
                        expires_at_ms: row.get(2)?,
                        created_at_ms: row.get(3)?,
                    },
                    decision: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub fn store_challenge(
    engine: &PersistenceEngine,
    challenge: &PrivateEgressChallengePayload,
) -> Result<(), String> {
    let binding = &challenge.binding;
    let source_names_json =
        serde_json::to_string(&challenge.source_names).map_err(|error| error.to_string())?;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO private_egress_confirmation_challenges (
                challenge_id,session_id,turn_id,generation_token,
                destination_provider_id,destination_model_id,source_digest,
                allowed_representation,representation_digest,source_names_json,
                decision,expires_at_ms,created_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending',?11,?12)
             ON CONFLICT(session_id,turn_id,generation_token) DO UPDATE SET
                challenge_id=excluded.challenge_id,
                destination_provider_id=excluded.destination_provider_id,
                destination_model_id=excluded.destination_model_id,
                source_digest=excluded.source_digest,
                allowed_representation=excluded.allowed_representation,
                representation_digest=excluded.representation_digest,
                source_names_json=excluded.source_names_json,
                decision='pending',expires_at_ms=excluded.expires_at_ms,
                decided_at_ms=NULL,consumed_at_ms=NULL,created_at_ms=excluded.created_at_ms",
            params![
                challenge.challenge_id,
                binding.session_id,
                binding.turn_id,
                binding.generation_token,
                binding.destination_provider_id,
                binding.destination_model_id,
                binding.source_digest,
                binding.allowed_representation,
                binding.representation_digest,
                source_names_json,
                challenge.expires_at_ms,
                challenge.created_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn consume_challenge(
    engine: &PersistenceEngine,
    challenge: &PrivateEgressChallengePayload,
    consumed_at_ms: i64,
) -> Result<bool, String> {
    let binding = &challenge.binding;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE private_egress_confirmation_challenges
             SET decision='consumed',consumed_at_ms=?2
             WHERE challenge_id=?1 AND decision='approved' AND expires_at_ms>=?2
               AND session_id=?3 AND turn_id=?4 AND generation_token=?5
               AND destination_provider_id=?6 AND destination_model_id=?7
               AND source_digest=?8 AND allowed_representation=?9
               AND representation_digest=?10",
            params![
                challenge.challenge_id,
                consumed_at_ms,
                binding.session_id,
                binding.turn_id,
                binding.generation_token,
                binding.destination_provider_id,
                binding.destination_model_id,
                binding.source_digest,
                binding.allowed_representation,
                binding.representation_digest,
            ],
        )
        .map(|count| count == 1)
        .map_err(|error| error.to_string())
}

pub fn has_consumed_turn_approval(
    engine: &PersistenceEngine,
    binding: &PrivateEgressChallengeBinding,
    now_ms: i64,
) -> Result<bool, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM private_egress_confirmation_challenges
                WHERE session_id=?1 AND turn_id=?2 AND generation_token=?3
                  AND destination_provider_id=?4 AND destination_model_id=?5
                  AND allowed_representation=?6 AND decision='consumed'
                  AND expires_at_ms>=?7
             )",
            params![
                binding.session_id,
                binding.turn_id,
                binding.generation_token,
                binding.destination_provider_id,
                binding.destination_model_id,
                binding.allowed_representation,
                now_ms,
            ],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

impl PrivateEgressStore for PersistenceEngine {
    fn authenticate_native_public_search(
        &self,
        claim: &NativePublicSearchClaim,
    ) -> Result<bool, String> {
        authenticate_native_public_search(self, claim)
    }

    fn store_private_egress_receipt(
        &self,
        receipt: &PrivateEgressReceiptPayload,
        signature_json: &str,
    ) -> Result<(), String> {
        store_receipt(self, receipt, signature_json)
    }

    fn consume_private_egress_receipt(
        &self,
        receipt: &PrivateEgressReceiptPayload,
        consumed_at_ms: i64,
    ) -> Result<bool, String> {
        consume_receipt(self, receipt, consumed_at_ms)
    }

    fn find_private_egress_challenge(
        &self,
        binding: &PrivateEgressChallengeBinding,
    ) -> Result<Option<StoredPrivateEgressChallenge>, String> {
        find_challenge(self, binding)
    }

    fn store_private_egress_challenge(
        &self,
        challenge: &PrivateEgressChallengePayload,
    ) -> Result<(), String> {
        store_challenge(self, challenge)
    }

    fn consume_private_egress_challenge(
        &self,
        challenge: &PrivateEgressChallengePayload,
        consumed_at_ms: i64,
    ) -> Result<bool, String> {
        consume_challenge(self, challenge, consumed_at_ms)
    }

    fn has_consumed_private_egress_turn_approval(
        &self,
        binding: &PrivateEgressChallengeBinding,
        now_ms: i64,
    ) -> Result<bool, String> {
        has_consumed_turn_approval(self, binding, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_metadata(claim: &mut NativePublicSearchClaim) -> serde_json::Value {
        let result_urls = serde_json::json!(["https://result.example/"]);
        let payload = serde_json::json!({
            "sessionId": claim.session_id,
            "turnId": "turn-search",
            "generationToken": "generation-search",
            "queryDigest": claim.query_digest,
            "contextDigest": claim.context_digest,
            "engine": claim.engine,
            "resultCount": claim.result_count,
            "retrievedPageCount": claim.source_urls.len(),
            "sourceUrls": claim.source_urls,
            "resultUrls": result_urls,
            "accessedAtUtc": claim.accessed_at_utc,
            "invocationIndex": claim.invocation_index,
        });
        claim.receipt_digest = crate::foundation::digest::sha256_hex(
            serde_json::to_string(&payload).unwrap().as_bytes(),
        );
        serde_json::json!({
            "eventKind": "sovereign_search_receipt",
            "checkpointKind": "verified_sovereign_search",
            "sessionId": claim.session_id,
            "turnId": "turn-search",
            "generationToken": "generation-search",
            "searchReceiptDigest": claim.receipt_digest,
            "searchInvocationIndex": claim.invocation_index,
            "queryDigest": claim.query_digest,
            "contextDigest": claim.context_digest,
            "searchEngine": claim.engine,
            "resultCount": claim.result_count,
            "retrievedPageCount": claim.source_urls.len(),
            "sourceUrls": claim.source_urls,
            "resultUrls": result_urls,
            "accessedAtUtc": claim.accessed_at_utc,
        })
    }

    #[test]
    fn public_status_requires_exact_native_receipt_provenance() {
        let mut claim = NativePublicSearchClaim {
            session_id: "session-search".to_string(),
            receipt_digest: String::new(),
            invocation_index: 1,
            query_digest: crate::foundation::digest::sha256_hex(b"official release"),
            context_digest: crate::foundation::digest::sha256_hex(b"verified context"),
            engine: "duckduckgo_lite_static".to_string(),
            result_count: 1,
            source_urls: vec!["https://official.example/".to_string()],
            accessed_at_utc: "2026-07-28T12:00:00.000Z".to_string(),
        };
        let metadata = native_metadata(&mut claim);
        assert!(authenticated_search_metadata(&metadata, &claim));

        let mut altered = claim.clone();
        altered.source_urls = vec!["https://private.example/".to_string()];
        assert!(!authenticated_search_metadata(&metadata, &altered));

        let mut altered_context = claim;
        altered_context.context_digest =
            crate::foundation::digest::sha256_hex(b"altered context with the same source URLs");
        assert!(!authenticated_search_metadata(&metadata, &altered_context));
    }
}
