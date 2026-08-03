use super::{claim_value, verify_sha256_hex};
use chrono::DateTime;

pub(super) fn verify(claim: &str) -> Option<Result<(), String>> {
    let result = if claim.starts_with("decision_pack_file_verified=") {
        verify_decision_pack_file(claim)
    } else if claim.starts_with("decision_pack_analysis_verified=") {
        verify_decision_pack_analysis(claim)
    } else if claim.starts_with("calendar_event_created=") {
        verify_calendar_created(claim)
    } else if claim.starts_with("calendar_event_verified=") {
        verify_calendar_verified(claim)
    } else if claim.starts_with("mail_draft_saved=") {
        verify_mail_draft(claim)
    } else if claim.starts_with("decision_pack_postcondition_verified=") {
        verify_decision_pack_postcondition(claim)
    } else if claim.starts_with("release_recovery_agenda_verified=") {
        verify_release_recovery_agenda(claim)
    } else if claim.starts_with("release_recovery_postcondition_verified=") {
        verify_release_recovery_postcondition(claim)
    } else {
        return None;
    };
    Some(result)
}

fn verify_decision_pack_file(claim: &str) -> Result<(), String> {
    require_value(claim, "decision_pack_file_verified", "true")?;
    require_one_of(
        claim,
        "kind",
        &["workbook", "presentation", "pdf", "sources"],
    )?;
    require_digest(claim, "path_sha256")?;
    require_digest(claim, "sha256")?;
    require_positive_usize(claim, "byte_count")?;
    require_exact_keys(
        claim,
        &[
            "decision_pack_file_verified",
            "kind",
            "path_sha256",
            "sha256",
            "byte_count",
        ],
    )
}

fn verify_decision_pack_analysis(claim: &str) -> Result<(), String> {
    require_value(claim, "decision_pack_analysis_verified", "true")?;
    require_digest(claim, "analysis_sha256")?;
    require_positive_usize(claim, "official_web_sources")?;
    require_exact_keys(
        claim,
        &[
            "decision_pack_analysis_verified",
            "analysis_sha256",
            "official_web_sources",
        ],
    )
}

fn verify_calendar_created(claim: &str) -> Result<(), String> {
    let created = require_bool(claim, "calendar_event_created")?;
    let reused = require_bool(claim, "reused_existing")?;
    if created == reused {
        return Err(
            "Calendar evidence must prove exactly one of a new or reused existing event."
                .to_string(),
        );
    }
    require_exact_keys(claim, &["calendar_event_created", "reused_existing"])
}

fn verify_calendar_verified(claim: &str) -> Result<(), String> {
    require_value(claim, "calendar_event_verified", "true")?;
    require_value(claim, "exists", "true")?;
    require_digest(claim, "event_id_sha256")?;
    require_exact_keys(
        claim,
        &["calendar_event_verified", "exists", "event_id_sha256"],
    )
}

fn verify_mail_draft(claim: &str) -> Result<(), String> {
    require_value(claim, "mail_draft_saved", "true")?;
    require_value(claim, "sent", "false")?;
    require_bool(claim, "reused_existing")?;
    for field in ["draft_id_sha256", "subject_sha256", "body_sha256"] {
        require_digest(claim, field)?;
    }
    require_exact_keys(
        claim,
        &[
            "mail_draft_saved",
            "sent",
            "reused_existing",
            "draft_id_sha256",
            "subject_sha256",
            "body_sha256",
        ],
    )
}

fn verify_decision_pack_postcondition(claim: &str) -> Result<(), String> {
    require_value(claim, "decision_pack_postcondition_verified", "true")?;
    require_value(claim, "file_count", "4")?;
    require_value(claim, "calendar_exact_match_count", "1")?;
    require_value(claim, "mail_exact_match_count", "1")?;
    require_digest(claim, "evidence_sha256")?;
    require_exact_keys(
        claim,
        &[
            "decision_pack_postcondition_verified",
            "file_count",
            "calendar_exact_match_count",
            "mail_exact_match_count",
            "evidence_sha256",
        ],
    )
}

fn verify_release_recovery_agenda(claim: &str) -> Result<(), String> {
    require_value(claim, "release_recovery_agenda_verified", "true")?;
    for field in ["output_sha256", "input_sha256", "path_sha256"] {
        require_digest(claim, field)?;
    }
    require_value(claim, "agenda_item_count", "5")?;
    let start = claim_value(claim, "start_date")
        .ok_or_else(|| "Verified evidence is missing start_date.".to_string())
        .and_then(|value| {
            DateTime::parse_from_rfc3339(value)
                .map_err(|_| "Verified evidence start_date must be RFC 3339.".to_string())
        })?;
    let end = claim_value(claim, "end_date")
        .ok_or_else(|| "Verified evidence is missing end_date.".to_string())
        .and_then(|value| {
            DateTime::parse_from_rfc3339(value)
                .map_err(|_| "Verified evidence end_date must be RFC 3339.".to_string())
        })?;
    if end.signed_duration_since(start).num_minutes() != 30 {
        return Err("Release recovery evidence must bind exactly 30 minutes.".to_string());
    }
    require_exact_keys(
        claim,
        &[
            "release_recovery_agenda_verified",
            "output_sha256",
            "input_sha256",
            "path_sha256",
            "agenda_item_count",
            "start_date",
            "end_date",
        ],
    )
}

fn verify_release_recovery_postcondition(claim: &str) -> Result<(), String> {
    require_value(claim, "release_recovery_postcondition_verified", "true")?;
    require_value(claim, "file_count", "1")?;
    require_value(claim, "calendar_exact_match_count", "1")?;
    require_value(claim, "mail_exact_match_count", "1")?;
    require_value(claim, "sent_match_count", "0")?;
    require_digest(claim, "evidence_sha256")?;
    require_exact_keys(
        claim,
        &[
            "release_recovery_postcondition_verified",
            "file_count",
            "calendar_exact_match_count",
            "mail_exact_match_count",
            "sent_match_count",
            "evidence_sha256",
        ],
    )
}

fn require_value(claim: &str, field: &str, expected: &str) -> Result<(), String> {
    match claim_value(claim, field) {
        Some(value) if value == expected => Ok(()),
        Some(value) => Err(format!(
            "Verified evidence {field} must be {expected}, not {value}."
        )),
        None => Err(format!("Verified evidence is missing {field}.")),
    }
}

fn require_bool(claim: &str, field: &str) -> Result<bool, String> {
    match claim_value(claim, field) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(format!("Verified evidence {field} must be a boolean.")),
        None => Err(format!("Verified evidence is missing {field}.")),
    }
}

fn require_one_of(claim: &str, field: &str, allowed: &[&str]) -> Result<(), String> {
    match claim_value(claim, field) {
        Some(value) if allowed.contains(&value) => Ok(()),
        Some(value) => Err(format!(
            "Verified evidence {field} is unsupported: {value}."
        )),
        None => Err(format!("Verified evidence is missing {field}.")),
    }
}

fn require_digest(claim: &str, field: &str) -> Result<(), String> {
    let value = claim_value(claim, field)
        .ok_or_else(|| format!("Verified evidence is missing {field}."))?;
    verify_sha256_hex(field, value)
}

fn require_positive_usize(claim: &str, field: &str) -> Result<(), String> {
    match claim_value(claim, field).and_then(|value| value.parse::<usize>().ok()) {
        Some(value) if value > 0 => Ok(()),
        _ => Err(format!(
            "Verified evidence {field} must be greater than zero."
        )),
    }
}

fn require_exact_keys(claim: &str, expected: &[&str]) -> Result<(), String> {
    let mut keys = Vec::new();
    for part in claim.split_whitespace() {
        let Some((key, _)) = part.split_once('=') else {
            return Err("Verified evidence contains malformed fields.".to_string());
        };
        if keys.contains(&key) {
            return Err(format!("Verified evidence repeats {key}."));
        }
        keys.push(key);
    }
    if keys.len() == expected.len() && keys.iter().all(|key| expected.contains(key)) {
        return Ok(());
    }
    Err("Verified evidence contains fields outside its signed contract.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn accepts_every_decision_pack_runtime_claim_family() {
        let claims = [
            format!("decision_pack_file_verified=true kind=workbook path_sha256={HASH} sha256={HASH} byte_count=143750"),
            format!("decision_pack_analysis_verified=true analysis_sha256={HASH} official_web_sources=1"),
            "calendar_event_created=true reused_existing=false".to_string(),
            format!("calendar_event_verified=true exists=true event_id_sha256={HASH}"),
            format!("mail_draft_saved=true sent=false reused_existing=false draft_id_sha256={HASH} subject_sha256={HASH} body_sha256={HASH}"),
            format!("decision_pack_postcondition_verified=true file_count=4 calendar_exact_match_count=1 mail_exact_match_count=1 evidence_sha256={HASH}"),
        ];
        for claim in claims {
            assert_eq!(verify(&claim), Some(Ok(())), "claim: {claim}");
        }
    }

    #[test]
    fn accepts_every_release_recovery_runtime_claim_family() {
        let claims = [
            format!("release_recovery_agenda_verified=true output_sha256={HASH} input_sha256={HASH} path_sha256={HASH} agenda_item_count=5 start_date=2026-07-21T13:30:00-04:00 end_date=2026-07-21T14:00:00-04:00"),
            format!("release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=0 evidence_sha256={HASH}"),
        ];
        for claim in claims {
            assert_eq!(verify(&claim), Some(Ok(())), "claim: {claim}");
        }
    }

    #[test]
    fn rejects_false_completion_tampering_and_unknown_fields() {
        let sent = format!("mail_draft_saved=true sent=true reused_existing=false draft_id_sha256={HASH} subject_sha256={HASH} body_sha256={HASH}");
        assert!(verify(&sent).unwrap().is_err());
        let wrong_count = format!("decision_pack_postcondition_verified=true file_count=3 calendar_exact_match_count=1 mail_exact_match_count=1 evidence_sha256={HASH}");
        assert!(verify(&wrong_count).unwrap().is_err());
        let extra =
            format!("calendar_event_verified=true exists=true event_id_sha256={HASH} trusted=true");
        assert!(verify(&extra).unwrap().is_err());

        let wrong_item_count = format!("release_recovery_agenda_verified=true output_sha256={HASH} input_sha256={HASH} path_sha256={HASH} agenda_item_count=4 start_date=2026-07-21T13:30:00-04:00 end_date=2026-07-21T14:00:00-04:00");
        assert!(verify(&wrong_item_count).unwrap().is_err());
        let wrong_duration = format!("release_recovery_agenda_verified=true output_sha256={HASH} input_sha256={HASH} path_sha256={HASH} agenda_item_count=5 start_date=2026-07-21T13:30:00-04:00 end_date=2026-07-21T14:30:00-04:00");
        assert!(verify(&wrong_duration).unwrap().is_err());
        let reversed = format!("release_recovery_agenda_verified=true output_sha256={HASH} input_sha256={HASH} path_sha256={HASH} agenda_item_count=5 start_date=2026-07-21T14:00:00-04:00 end_date=2026-07-21T13:30:00-04:00");
        assert!(verify(&reversed).unwrap().is_err());
        let uppercase = format!("release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=0 evidence_sha256={}", HASH.to_ascii_uppercase());
        assert!(verify(&uppercase).unwrap().is_err());
        let sent_copy = format!("release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=1 evidence_sha256={HASH}");
        assert!(verify(&sent_copy).unwrap().is_err());
        let duplicate = format!("release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=0 evidence_sha256={HASH} file_count=1");
        assert!(verify(&duplicate).unwrap().is_err());
    }
}
