use crate::artifact_builder::{read_signed_ark_artifact, SignedArkArtifact};
use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactAuditRequest {
    pub artifact_paths: Option<Vec<String>>,
    pub focus: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactAuditFinding {
    pub finding_kind: String,
    pub summary: String,
    pub artifact_hashes: Vec<String>,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactAuditReport {
    pub report_id: String,
    pub focus: String,
    pub artifact_count: usize,
    pub inherited_premises: Vec<String>,
    pub findings: Vec<ArtifactAuditFinding>,
    pub report_path: String,
    pub signature: SignatureBlock,
}

#[derive(Debug, Serialize)]
pub struct ArtifactAuditError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

pub fn audit_ark_artifacts_sync(
    request: ArtifactAuditRequest,
    identity: &SovereignIdentity,
) -> Result<ArtifactAuditReport, ArtifactAuditError> {
    let focus = request
        .focus
        .unwrap_or_else(|| "systemic procedural drift".to_string());
    let paths = match request.artifact_paths {
        Some(paths) => paths.into_iter().map(PathBuf::from).collect::<Vec<_>>(),
        None => discover_ark_json_artifacts()?,
    };
    let mut artifacts = Vec::new();
    for path in paths {
        let artifact = read_signed_ark_artifact(&guard_ark_path(&path)?)
            .map_err(|error| ArtifactAuditError::invalid(error))?;
        artifacts.push(artifact);
    }
    if artifacts.len() < 2 {
        return Err(ArtifactAuditError::invalid(
            "Comparative audit requires at least two signed Ark JSON artifacts.".to_string(),
        ));
    }

    let findings = detect_patterns(&artifacts);
    let inherited_premises = artifacts
        .iter()
        .flat_map(|artifact| {
            artifact
                .distilled_findings
                .iter()
                .take(3)
                .map(|finding| format!("{}: {}", artifact.artifact_hash, finding))
        })
        .collect::<Vec<_>>();
    let report_id = format!("artifact-audit-{}", unix_time_ms());
    let report_payload = serde_json::json!({
        "report_id": report_id,
        "focus": focus,
        "artifact_hashes": artifacts.iter().map(|artifact| artifact.artifact_hash.clone()).collect::<Vec<_>>(),
        "inherited_premises": inherited_premises,
        "findings": findings,
    });
    let signature = identity
        .sign_payload(&report_payload.to_string())
        .map_err(|error| ArtifactAuditError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })?;
    let report_path = project_root()
        .join("ark")
        .join(format!("{report_id}-anomalous-pattern-report.json"));
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).map_err(|error| ArtifactAuditError::io(error.to_string()))?;
    }
    let report = ArtifactAuditReport {
        report_id,
        focus,
        artifact_count: artifacts.len(),
        inherited_premises,
        findings,
        report_path: report_path.to_string_lossy().to_string(),
        signature,
    };
    let report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| ArtifactAuditError::invalid(error.to_string()))?;
    fs::write(&report_path, &report_bytes)
        .map_err(|error| ArtifactAuditError::io(error.to_string()))?;
    let persisted =
        fs::read(&report_path).map_err(|error| ArtifactAuditError::io(error.to_string()))?;
    if persisted != report_bytes {
        return Err(ArtifactAuditError::io(
            "Artifact audit report verification failed after write.".to_string(),
        ));
    }
    Ok(report)
}

#[tauri::command]
pub async fn audit_ark_artifacts(
    request: ArtifactAuditRequest,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ArtifactAuditReport, ArtifactAuditError> {
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || audit_ark_artifacts_sync(request, &identity))
        .await
        .map_err(|error| ArtifactAuditError::io(error.to_string()))?
}

fn detect_patterns(artifacts: &[SignedArkArtifact]) -> Vec<ArtifactAuditFinding> {
    let mut term_to_hashes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for artifact in artifacts {
        for finding in &artifact.distilled_findings {
            for term in normalized_terms(finding) {
                term_to_hashes
                    .entry(term)
                    .or_default()
                    .insert(artifact.artifact_hash.clone());
            }
        }
    }

    let mut findings = term_to_hashes
        .into_iter()
        .filter(|(_, hashes)| hashes.len() >= 3)
        .take(8)
        .map(|(term, hashes)| ArtifactAuditFinding {
            finding_kind: "recurring_anomaly".to_string(),
            summary: format!(
                "Recurring procedural signal '{term}' appears across {} signed artifacts.",
                hashes.len()
            ),
            artifact_hashes: hashes.into_iter().collect(),
            severity: "high".to_string(),
        })
        .collect::<Vec<_>>();

    findings.extend(detect_contradictions(artifacts));
    if findings.is_empty() {
        findings.push(ArtifactAuditFinding {
            finding_kind: "no_systemic_pattern".to_string(),
            summary: "No recurring anomaly or contradiction crossed the comparative threshold."
                .to_string(),
            artifact_hashes: artifacts
                .iter()
                .map(|artifact| artifact.artifact_hash.clone())
                .collect(),
            severity: "informational".to_string(),
        });
    }
    findings
}

fn detect_contradictions(artifacts: &[SignedArkArtifact]) -> Vec<ArtifactAuditFinding> {
    let mut findings = Vec::new();
    for artifact in artifacts {
        let text = artifact.distilled_findings.join(" ").to_lowercase();
        if text.contains("verified") && text.contains("unverified") {
            findings.push(ArtifactAuditFinding {
                finding_kind: "internal_contradiction".to_string(),
                summary: format!(
                    "Artifact {} contains both verified and unverified procedural claims.",
                    artifact.artifact_hash
                ),
                artifact_hashes: vec![artifact.artifact_hash.clone()],
                severity: "medium".to_string(),
            });
        }
    }
    findings
}

fn normalized_terms(input: &str) -> Vec<String> {
    input
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| {
            term.len() > 5
                && !matches!(
                    term.as_str(),
                    "claim" | "source" | "operation" | "completed" | "distilled"
                )
        })
        .collect()
}

fn discover_ark_json_artifacts() -> Result<Vec<PathBuf>, ArtifactAuditError> {
    let ark_dir = project_root().join("ark");
    let entries = fs::read_dir(&ark_dir).map_err(|error| {
        ArtifactAuditError::io(format!(
            "Unable to inspect the private Ark directory {}: {error}",
            ark_dir.display()
        ))
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ArtifactAuditError::io(format!(
                "Unable to inspect an entry in the private Ark directory: {error}"
            ))
        })?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-artifact.json"))
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn guard_ark_path(path: &Path) -> Result<PathBuf, ArtifactAuditError> {
    guard_ark_path_under(&project_root(), path)
}

fn guard_ark_path_under(root: &Path, path: &Path) -> Result<PathBuf, ArtifactAuditError> {
    let ark_root = fs::canonicalize(root.join("ark")).map_err(|_| {
        ArtifactAuditError::invalid("The private Ark directory is unavailable.".to_string())
    })?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let link_metadata = fs::symlink_metadata(&candidate).map_err(|_| {
        ArtifactAuditError::invalid("The requested Ark artifact is unavailable.".to_string())
    })?;
    if link_metadata.file_type().is_symlink() {
        return Err(ArtifactAuditError::invalid(
            "Symbolic-link Ark artifacts are not accepted.".to_string(),
        ));
    }
    let canonical = fs::canonicalize(&candidate).map_err(|_| {
        ArtifactAuditError::invalid("The requested Ark artifact is unavailable.".to_string())
    })?;
    if !canonical.starts_with(&ark_root) || canonical == ark_root || !canonical.is_file() {
        return Err(ArtifactAuditError::invalid(
            "Artifact audit is limited to canonical files inside the private Ark directory."
                .to_string(),
        ));
    }
    Ok(canonical)
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

impl ArtifactAuditError {
    fn invalid(message: String) -> Self {
        Self {
            code: "artifact_audit_invalid",
            boundary: "ArtifactAuditor",
            message,
        }
    }

    fn io(message: String) -> Self {
        Self {
            code: "artifact_audit_io",
            boundary: "ArtifactAuditor",
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_root(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        std::env::temp_dir().join(format!(
            "oomu-artifact-audit-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn ark_guard_canonicalizes_before_checking_containment() {
        let root = test_root("containment");
        let ark = root.join("ark");
        fs::create_dir_all(&ark).unwrap();
        let approved = ark.join("approved-artifact.json");
        let outside = root.join("outside-artifact.json");
        fs::write(&approved, "{}").unwrap();
        fs::write(&outside, "{}").unwrap();

        assert_eq!(
            guard_ark_path_under(&root, Path::new("ark/approved-artifact.json")).unwrap(),
            fs::canonicalize(&approved).unwrap()
        );
        assert!(guard_ark_path_under(&root, Path::new("ark/../outside-artifact.json")).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ark_guard_rejects_symlink_artifacts() {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink");
        let ark = root.join("ark");
        fs::create_dir_all(&ark).unwrap();
        let outside = root.join("outside-artifact.json");
        fs::write(&outside, "{}").unwrap();
        symlink(&outside, ark.join("linked-artifact.json")).unwrap();

        assert!(guard_ark_path_under(&root, Path::new("ark/linked-artifact.json")).is_err());

        fs::remove_dir_all(root).unwrap();
    }
}
