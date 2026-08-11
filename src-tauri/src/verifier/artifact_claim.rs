const PRESENTATION: &str = "presentation_artifact_created";
const WORKBOOK: &str = "workbook_artifact_created";

pub(super) fn verify(claim: &str) -> Option<Result<(), String>> {
    let tokens = claim.split_ascii_whitespace().collect::<Vec<_>>();
    let kind = *tokens.first()?;
    if !matches!(kind, PRESENTATION | WORKBOOK) {
        return None;
    }
    Some(verify_tokens(kind, &tokens))
}

fn verify_tokens(kind: &str, tokens: &[&str]) -> Result<(), String> {
    let [_, artifact, task_run, revision, export_ready] = tokens else {
        return Err(format!("{kind} claim must contain exactly four fields."));
    };
    verify_identifier(artifact, "artifact_id", "artifact_", kind)?;
    verify_identifier(task_run, "task_run_id", "taskrun_", kind)?;
    let revision = field_value(revision, "revision", kind)?
        .parse::<u32>()
        .map_err(|_| format!("{kind} revision must be a positive integer."))?;
    if revision == 0 {
        return Err(format!("{kind} revision must be a positive integer."));
    }
    if !matches!(
        field_value(export_ready, "export_ready", kind)?,
        "true" | "false"
    ) {
        return Err(format!("{kind} export_ready must be true or false."));
    }
    Ok(())
}

fn verify_identifier(token: &str, field: &str, prefix: &str, kind: &str) -> Result<(), String> {
    let value = field_value(token, field, kind)?;
    let suffix = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{kind} {field} has the wrong identifier type."))?;
    if suffix.is_empty()
        || suffix.len() > 80
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("{kind} {field} is not a bounded identifier."));
    }
    Ok(())
}

fn field_value<'a>(token: &'a str, field: &str, kind: &str) -> Result<&'a str, String> {
    token
        .strip_prefix(&format!("{field}="))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{kind} claim is missing {field}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_native_artifact_claims_are_recognized() {
        for kind in [PRESENTATION, WORKBOOK] {
            let claim = format!(
                "{kind} artifact_id=artifact_a653bd6e-ba21-4056-b95a-4b7d18b071bd task_run_id=taskrun_8d3d22b0-b98c-4fb5-90e6-2d017f67f104 revision=1 export_ready=true"
            );
            assert_eq!(verify(&claim), Some(Ok(())));
        }
    }

    #[test]
    fn malformed_native_artifact_claims_fail_closed() {
        for claim in [
            "presentation_artifact_created artifact_id=wrong task_run_id=taskrun_ok revision=1 export_ready=true",
            "presentation_artifact_created artifact_id=artifact_ok task_run_id=taskrun_ok revision=0 export_ready=true",
            "presentation_artifact_created artifact_id=artifact_ok task_run_id=taskrun_ok revision=1 export_ready=maybe",
            "presentation_artifact_created artifact_id=artifact_ok task_run_id=taskrun_ok revision=1 export_ready=true extra=true",
        ] {
            assert!(verify(claim).is_some_and(|result| result.is_err()), "{claim}");
        }
    }
}
