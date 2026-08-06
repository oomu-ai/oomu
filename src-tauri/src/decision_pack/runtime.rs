use super::{
    analysis::build_analysis, request_digest, research::research_official_sources,
    DecisionPackToolRequest, VerifiedInput, MAX_INPUT_FILE_BYTES, MAX_TOTAL_INPUT_BYTES,
};
use crate::{
    artifacts::{
        decision_pack::build_decision_pack,
        presentations::{
            create_presentation_internal, export_presentation_revision_to_approved_path,
            CreatePresentationRequest,
        },
        workbooks::{
            create_workbook_internal, export_workbook_revision_to_approved_path,
            CreateWorkbookRequest,
        },
        CreateArtifactRequest,
    },
    foundation::digest::{sha256_file_hex, sha256_hex},
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::require_agent_runtime_task, task_tool_runtime::TaskToolExecutionContext,
    },
};
use rand_core::{OsRng, RngCore};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
#[cfg(target_os = "macos")]
use std::{ffi::CString, os::unix::ffi::OsStrExt};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const RECEIPT_SCHEMA_VERSION: u32 = 1;
const EVIDENCE_EVENT: &str = "decision_pack.evidence_bound";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifiedOutput {
    kind: &'static str,
    path: String,
    sha256: String,
    byte_count: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingDecisionPackReceipt {
    schema_version: u32,
    analysis_sha256: String,
    recommendation: String,
    email_summary: String,
    files: Vec<ExistingDecisionPackFileReceipt>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingDecisionPackFileReceipt {
    kind: String,
    path: String,
    sha256: String,
    byte_count: u64,
}

struct DecisionPackPublication {
    final_directory: PathBuf,
    owned_directory: PathBuf,
    device: u64,
    inode: u64,
    committed: bool,
}

impl DecisionPackPublication {
    fn begin(
        output_binding: &crate::shield_gate::ApprovedExternalDirectoryBinding,
    ) -> Result<Self, String> {
        let created = crate::shield_gate::create_bound_approved_external_directory(output_binding)
            .map_err(|error| format!("{} ({})", error.message, error.code))?;
        let mut publication = Self::from_owned_directory(created)?;
        let staging = unique_staging_directory(&publication.final_directory)?;
        publication.relocate_owned_directory(&staging)?;
        sync_parent_directory(&staging)?;
        Ok(publication)
    }

    fn from_owned_directory(final_directory: PathBuf) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(&final_directory)
            .map_err(|_| "The approved output folder could not be checked.".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("The approved output folder is not a real directory.".to_string());
        }
        Ok(Self {
            owned_directory: final_directory.clone(),
            final_directory,
            device: metadata.dev(),
            inode: metadata.ino(),
            committed: false,
        })
    }

    fn owned_directory(&self) -> &Path {
        &self.owned_directory
    }

    fn publish(&mut self) -> Result<(), String> {
        sync_directory(&self.owned_directory)?;
        let final_directory = self.final_directory.clone();
        self.relocate_owned_directory(&final_directory)?;
        sync_parent_directory(&self.final_directory)?;
        Ok(())
    }

    fn commit(&mut self) {
        self.committed = true;
    }

    fn relocate_owned_directory(&mut self, destination: &Path) -> Result<(), String> {
        require_owned_directory(
            &self.owned_directory,
            self.device,
            self.inode,
            "The decision-pack staging folder changed before publication.",
        )?;
        rename_directory_no_replace(&self.owned_directory, destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "The approved decision-pack output folder appeared before publication. Nothing was replaced."
                    .to_string()
            } else {
                format!("The decision-pack folder could not be published safely: {error}")
            }
        })?;
        self.owned_directory = destination.to_path_buf();
        require_owned_directory(
            &self.owned_directory,
            self.device,
            self.inode,
            "The decision-pack folder changed during publication.",
        )
    }

    fn cleanup_owned_directory(&mut self) -> Result<(), String> {
        match fs::symlink_metadata(&self.owned_directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                return Err(
                    "The failed decision-pack staging folder could not be inspected.".to_string(),
                )
            }
            Ok(_) => {}
        }
        require_owned_directory(
            &self.owned_directory,
            self.device,
            self.inode,
            "The failed decision-pack staging folder no longer matches this run; it was not removed.",
        )?;
        fs::remove_dir_all(&self.owned_directory).map_err(|_| {
            "The failed decision-pack staging folder could not be removed completely.".to_string()
        })?;
        sync_parent_directory(&self.owned_directory)
    }
}

impl Drop for DecisionPackPublication {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = self.cleanup_owned_directory() {
                eprintln!("DECISION_PACK_STAGE_CLEANUP_FAILED error={error}");
            }
        }
    }
}

fn unique_staging_directory(final_directory: &Path) -> Result<PathBuf, String> {
    let parent = final_directory.parent().ok_or_else(|| {
        "The approved decision-pack output folder has no parent directory.".to_string()
    })?;
    for _ in 0..8 {
        let mut token = [0_u8; 16];
        OsRng.fill_bytes(&mut token);
        let candidate = parent.join(format!(".oomu-decision-pack-stage-{}", hex::encode(token)));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(_) => {
                return Err(
                    "The approved output parent could not be inspected for staging.".to_string(),
                )
            }
        }
    }
    Err("OOMU could not allocate a unique decision-pack staging folder.".to_string())
}

fn require_owned_directory(
    path: &Path,
    expected_device: u64,
    expected_inode: u64,
    message: &str,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| message.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.dev() != expected_device
        || metadata.ino() != expected_inode
    {
        return Err(message.to_string());
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| {
            "The decision-pack directory could not be opened for synchronization.".to_string()
        })?;
    directory
        .sync_all()
        .map_err(|_| "The decision-pack directory could not be synchronized.".to_string())
}

fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "The decision-pack directory has no parent to synchronize.".to_string())?;
    sync_directory(parent)
}

#[cfg(target_os = "macos")]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn rename_directory_no_replace(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace directory publication is unavailable on this platform",
    ))
}

pub(super) async fn execute(
    context: TaskToolExecutionContext<'_>,
    request: DecisionPackToolRequest,
) -> Result<ExecuteCommandResponse, String> {
    let execution_id = context
        .execution_id
        .ok_or_else(|| "Creating a decision pack requires an active approved Task.".to_string())?;
    let task = require_agent_runtime_task(context.persistence, execution_id)?;
    let app = context
        .app
        .ok_or_else(|| "Creating a decision pack requires the OOMU desktop app.".to_string())?;
    let plan_id = context
        .plan_id
        .ok_or_else(|| "Decision-pack research requires its approved action plan.".to_string())?;
    let objective = context
        .objective
        .ok_or_else(|| "Decision-pack research requires its original objective.".to_string())?;
    let output_binding = request.output_binding.as_ref().ok_or_else(|| {
        "Decision-pack output approval binding is unavailable. Nothing was changed.".to_string()
    })?;
    if request.locale != "en-US" {
        return Err(
            "The native decision-pack builder currently requires locale en-US. Nothing was changed."
                .to_string(),
        );
    }
    if !analysis_scope_is_supported(&request.analysis_instructions) {
        return Err(
            "The decision-pack analysis must explicitly require amount reconciliation, margin assessment, and exception identification. Nothing was changed."
                .to_string(),
        );
    }

    let inputs = read_verified_inputs(&request)?;
    if output_binding.existed_when_bound() {
        return reuse_existing_decision_pack(&request, objective, &inputs, context.persistence);
    }
    for input in &inputs {
        crate::tasks::record_domain_event(
            context.persistence,
            &task.task_run_id,
            "decision_pack.source_read_verified",
            EvidenceClass::VerifiedPostcondition,
            json!({"path":input.path,"sha256":input.sha256}),
        )?;
    }
    let research = research_official_sources(
        &request,
        plan_id,
        objective,
        inputs.len(),
        context.session_id,
        app,
        context.persistence,
    )
    .await?;
    let analysis = build_analysis(&request.title, &inputs, research.claims, research.gaps)?;
    let analysis_json = serde_json::to_vec(&analysis).map_err(|error| error.to_string())?;
    let analysis_sha256 = sha256_hex(&analysis_json);
    let sequence = crate::tools::task_runtime::record_event_with_sequence(
        context.persistence,
        &task.task_run_id,
        EVIDENCE_EVENT,
        EvidenceClass::VerifiedPostcondition,
        json!({
            "analysisSha256": analysis_sha256,
            "requestSha256": request_digest(&request)?,
            "inputFiles": inputs.iter().map(|input| json!({"path":input.path,"sha256":input.sha256})).collect::<Vec<_>>(),
            "webSources": analysis.web_claims.iter().map(|claim| json!({
                "subject":claim.subject,
                "url":claim.url,
                "effectiveDate":claim.effective_date,
                "dateEvidenceType":claim.date_evidence_type,
                "authority":claim.authority,
                "evidenceDigest":claim.evidence_digest,
                "accessedAt":claim.accessed_at
            })).collect::<Vec<_>>(),
            "researchGaps": analysis.research_gaps
        }),
    )?;
    let mut artifacts = build_decision_pack(&analysis)?;
    artifacts.bind_verified_task_evidence(
        EVIDENCE_EVENT,
        &format!("task-event:{}:{sequence}", task.task_run_id),
        &format!("canonical analysis sha256 {analysis_sha256}"),
    )?;
    artifacts.sources_markdown =
        local_source_preamble(&inputs, &analysis_sha256) + &artifacts.sources_markdown;

    let workbook_review = create_workbook_internal(
        CreateWorkbookRequest {
            project_id: task.project_id.clone(),
            task_id: task.task_id.clone(),
            task_run_id: task.task_run_id.clone(),
            workbook: artifacts.workbook,
        },
        context.persistence,
        context.identity,
        app,
    )
    .await
    .map_err(|error| format!("{} ({})", error.message, error.code))?;
    let presentation_review = create_presentation_internal(
        CreatePresentationRequest {
            project_id: task.project_id.clone(),
            task_id: task.task_id.clone(),
            task_run_id: task.task_run_id.clone(),
            title: artifacts.presentation.title.clone(),
            presentation: artifacts.presentation,
        },
        context.persistence,
        context.identity,
        app,
    )
    .await
    .map_err(|error| format!("{} ({})", error.message, error.code))?;
    let document_record = crate::artifacts::create_artifact_internal(
        CreateArtifactRequest {
            project_id: task.project_id.clone(),
            task_run_id: task.task_run_id.clone(),
            document: artifacts.document,
        },
        context.persistence,
        context.identity,
    )
    .await?;

    let mut publication = DecisionPackPublication::begin(output_binding)?;
    let staging_directory = publication.owned_directory().to_path_buf();
    let workbook_path = staging_directory.join(&request.outputs.workbook);
    let presentation_path = staging_directory.join(&request.outputs.presentation);
    let pdf_path = staging_directory.join(&request.outputs.pdf);
    let sources_path = staging_directory.join(&request.outputs.sources);
    require_new_outputs([&workbook_path, &presentation_path, &pdf_path, &sources_path])?;

    let workbook_export = export_workbook_revision_to_approved_path(
        &workbook_review.artifact_id,
        workbook_review.current_revision,
        &path_text(&workbook_path)?,
        context.persistence,
        context.identity,
        app,
    )
    .await
    .map_err(|error| format!("{} ({})", error.message, error.code))?;
    let presentation_export = export_presentation_revision_to_approved_path(
        &presentation_review.summary.presentation_id,
        presentation_review.summary.current_revision,
        &path_text(&presentation_path)?,
        context.persistence,
        context.identity,
        app,
    )
    .await
    .map_err(|error| format!("{} ({})", error.message, error.code))?;
    let pdf_export = crate::artifacts::export_verified_artifact_to_approved_path(
        &document_record.artifact_id,
        document_record.current_version,
        "pdf",
        &path_text(&pdf_path)?,
        context.persistence,
        context.identity,
    )?;
    write_new_markdown(&sources_path, &artifacts.sources_markdown)?;

    let staged_workbook = verify_output(
        "workbook",
        &workbook_path,
        Some(&workbook_export.sha256),
        b"PK",
    )?;
    let staged_presentation = verify_output(
        "presentation",
        &presentation_path,
        Some(&presentation_export.sha256),
        b"PK",
    )?;
    let staged_pdf = verify_output(
        "pdf",
        &pdf_path,
        Some(pdf_export.hashes.get("pdf").ok_or_else(|| {
            "The PDF export receipt did not include its verified hash.".to_string()
        })?),
        b"%PDF-",
    )?;
    let staged_sources = verify_output("sources", &sources_path, None, b"# Approved local inputs")?;

    verify_cross_format_semantics(
        &analysis,
        &workbook_path,
        &presentation_path,
        &pdf_path,
        &sources_path,
    )?;

    publication.publish()?;
    let output_directory = publication.owned_directory();
    let workbook_path = output_directory.join(&request.outputs.workbook);
    let presentation_path = output_directory.join(&request.outputs.presentation);
    let pdf_path = output_directory.join(&request.outputs.pdf);
    let sources_path = output_directory.join(&request.outputs.sources);
    let files = vec![
        verify_output(
            "workbook",
            &workbook_path,
            Some(&staged_workbook.sha256),
            b"PK",
        )?,
        verify_output(
            "presentation",
            &presentation_path,
            Some(&staged_presentation.sha256),
            b"PK",
        )?,
        verify_output("pdf", &pdf_path, Some(&staged_pdf.sha256), b"%PDF-")?,
        verify_output(
            "sources",
            &sources_path,
            Some(&staged_sources.sha256),
            b"# Approved local inputs",
        )?,
    ];
    sync_directory(output_directory)?;
    publication.commit();
    let message = json!({
        "schemaVersion": RECEIPT_SCHEMA_VERSION,
        "analysisSha256": analysis_sha256,
        "recommendation": analysis.recommendation,
        "emailSummary": analysis.email_summary,
        "files": files,
    })
    .to_string();
    let claims = files
        .iter()
        .map(|file| {
            format!(
                "CLAIM decision_pack_file_verified=true kind={} path_sha256={} sha256={} byte_count={}",
                file.kind,
                sha256_hex(file.path.as_bytes()),
                file.sha256,
                file.byte_count
            )
        })
        .chain(std::iter::once(format!(
            "CLAIM decision_pack_analysis_verified=true analysis_sha256={} official_web_sources={}",
            analysis_sha256,
            analysis.web_claims.len()
        )))
        .collect();
    Ok(ExecuteCommandResponse {
        operation: "create_decision_pack".to_string(),
        status: CommandStatus::Completed,
        message,
        metrics: None,
        claims,
        verified: true,
        model_used: None,
    })
}

fn reuse_existing_decision_pack(
    request: &DecisionPackToolRequest,
    objective: &str,
    inputs: &[VerifiedInput],
    persistence: &crate::db::PersistenceEngine,
) -> Result<ExecuteCommandResponse, String> {
    let candidates = persistence
        .completed_agent_action_outputs_for_objective("create_decision_pack", objective)?;
    for candidate in candidates {
        let Ok(mut output) = serde_json::from_str::<ExecuteCommandResponse>(&candidate) else {
            continue;
        };
        if output.operation != "create_decision_pack"
            || !output.verified
            || !matches!(output.status, CommandStatus::Completed)
        {
            continue;
        }
        let Ok(receipt) = serde_json::from_str::<ExistingDecisionPackReceipt>(&output.message)
        else {
            continue;
        };
        if verify_existing_decision_pack_receipt(request, inputs, &receipt).is_err() {
            continue;
        }
        output.claims =
            existing_decision_pack_claims(&receipt, "decision_pack_existing_receipt_reverified");
        output.model_used = None;
        return Ok(output);
    }
    if let Ok(receipt) = reconstruct_existing_decision_pack_receipt(request, inputs) {
        return Ok(ExecuteCommandResponse {
            operation: "create_decision_pack".to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&receipt).map_err(|error| error.to_string())?,
            metrics: None,
            claims: existing_decision_pack_claims(
                &receipt,
                "decision_pack_existing_artifacts_reverified",
            ),
            verified: true,
            model_used: None,
        });
    }
    Err(existing_output_unverified_error())
}

fn existing_decision_pack_claims(
    receipt: &ExistingDecisionPackReceipt,
    recovery_claim: &str,
) -> Vec<String> {
    receipt
        .files
        .iter()
        .map(|file| {
            format!(
                "CLAIM decision_pack_file_verified=true kind={} path_sha256={} sha256={} byte_count={}",
                file.kind,
                sha256_hex(file.path.as_bytes()),
                file.sha256,
                file.byte_count
            )
        })
        .chain([
            format!(
                "CLAIM decision_pack_analysis_verified=true analysis_sha256={} reused_existing=true",
                receipt.analysis_sha256
            ),
            format!("CLAIM {recovery_claim}=true file_count=4"),
        ])
        .collect()
}

fn reconstruct_existing_decision_pack_receipt(
    request: &DecisionPackToolRequest,
    inputs: &[VerifiedInput],
) -> Result<ExistingDecisionPackReceipt, String> {
    let output_directory = Path::new(&request.output_directory);
    let workbook_path = output_directory.join(&request.outputs.workbook);
    let presentation_path = output_directory.join(&request.outputs.presentation);
    let pdf_path = output_directory.join(&request.outputs.pdf);
    let sources_path = output_directory.join(&request.outputs.sources);
    let sources = fs::read_to_string(&sources_path)
        .map_err(|_| "The existing source ledger could not be reopened.".to_string())?;
    let recorded_analysis_sha256 = sources
        .lines()
        .find_map(|line| {
            line.strip_prefix("Canonical analysis SHA-256: `")
                .and_then(|value| value.strip_suffix('`'))
        })
        .filter(|value| is_sha256(value))
        .ok_or_else(|| "The existing source ledger has no valid analysis digest.".to_string())?;
    let (web_claims, research_gaps) = parse_existing_research_evidence(&sources)?;
    let analysis = build_analysis(&request.title, inputs, web_claims, research_gaps)?;
    let analysis_sha256 =
        sha256_hex(&serde_json::to_vec(&analysis).map_err(|error| error.to_string())?);
    if analysis_sha256 != recorded_analysis_sha256 {
        return Err(
            "The existing source ledger does not reproduce its canonical analysis.".to_string(),
        );
    }
    let expected_sources = local_source_preamble(inputs, &analysis_sha256)
        + &build_decision_pack(&analysis)?.sources_markdown;
    if sources != expected_sources {
        return Err("The existing source ledger changed after OOMU created it.".to_string());
    }
    verify_cross_format_semantics(
        &analysis,
        &workbook_path,
        &presentation_path,
        &pdf_path,
        &sources_path,
    )?;
    let files = [
        verify_output("workbook", &workbook_path, None, b"PK")?,
        verify_output("presentation", &presentation_path, None, b"PK")?,
        verify_output("pdf", &pdf_path, None, b"%PDF-")?,
        verify_output("sources", &sources_path, None, b"# Approved local inputs")?,
    ]
    .into_iter()
    .map(|file| ExistingDecisionPackFileReceipt {
        kind: file.kind.to_string(),
        path: file.path,
        sha256: file.sha256,
        byte_count: file.byte_count,
    })
    .collect();
    Ok(ExistingDecisionPackReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        analysis_sha256,
        recommendation: analysis.recommendation,
        email_summary: analysis.email_summary,
        files,
    })
}

fn parse_existing_research_evidence(
    sources: &str,
) -> Result<
    (
        Vec<crate::artifacts::decision_pack::WebClaim>,
        Vec<crate::artifacts::decision_pack::ResearchGap>,
    ),
    String,
> {
    use crate::{
        artifacts::decision_pack::{
            web_claim_evidence_digest, DateEvidenceType, ResearchGap, ResearchGapReason,
            SourceAuthority, SourceAuthorityClass, WebClaim,
        },
        decision_research_policy::{authority_profile_for_url, AuthorityClass},
    };

    let web_section = sources
        .split_once("## Current web sources\n\n")
        .and_then(|(_, remainder)| remainder.split_once("\n\n## Research gaps\n\n"))
        .ok_or_else(|| {
            "The existing source ledger is missing its research evidence.".to_string()
        })?;
    if web_section.0 == "No web claims were included in the canonical analysis.\n" {
        return Err("The existing source ledger has no verified web evidence.".to_string());
    }
    let claim_pattern = Regex::new(
        r"(?s)^\d+\. \*\*(.+?)\*\* — (.+?)  \n   Authority: (.+?) \((government|intergovernmental|registered first-party)\)  \n   Source: \[(.+?)\]\((.+?)\)  \n   Effective date: `([^`]+)` \((publication date|release date|observation date|updated date)\)  \n   Accessed: `([^`]+)`  \n   Evidence digest: `([0-9a-f]{64})`  \n   Canonical source: `decision-pack-web-claim-\d+`  \n   Canonical evidence: `canonical-analysis-web-claim-\d+`$",
    )
    .map_err(|error| error.to_string())?;
    let web_claims = web_section
        .0
        .trim_end()
        .split("\n\n")
        .map(|block| {
            let captures = claim_pattern.captures(block).ok_or_else(|| {
                "The existing web evidence is not in OOMU’s canonical format.".to_string()
            })?;
            let decoded = |index| decode_markdown_line(&captures[index]);
            let subject = decoded(1);
            let claim = decoded(2);
            let organization = decoded(3);
            let source_title = decoded(5);
            let url = captures[6].to_string();
            let profile = authority_profile_for_url(&url).ok_or_else(|| {
                "The existing web evidence no longer uses an approved official source.".to_string()
            })?;
            let class = match &captures[4] {
                "government" => SourceAuthorityClass::Government,
                "intergovernmental" => SourceAuthorityClass::Intergovernmental,
                "registered first-party" => SourceAuthorityClass::RegisteredFirstParty,
                _ => unreachable!("regex restricts authority class"),
            };
            let profile_class = match profile.class {
                AuthorityClass::Government => SourceAuthorityClass::Government,
                AuthorityClass::Intergovernmental => SourceAuthorityClass::Intergovernmental,
                AuthorityClass::RegisteredFirstParty => SourceAuthorityClass::RegisteredFirstParty,
            };
            if organization != profile.organization || class != profile_class {
                return Err(
                    "The existing web evidence authority no longer matches its official source."
                        .to_string(),
                );
            }
            let date_evidence_type = match &captures[8] {
                "publication date" => DateEvidenceType::PublicationDate,
                "release date" => DateEvidenceType::ReleaseDate,
                "observation date" => DateEvidenceType::ObservationDate,
                "updated date" => DateEvidenceType::UpdatedDate,
                _ => unreachable!("regex restricts date evidence type"),
            };
            let authority = SourceAuthority {
                profile_id: profile.id.to_string(),
                organization,
                class,
            };
            let effective_date = captures[7].to_string();
            let evidence_digest = captures[10].to_string();
            if evidence_digest
                != web_claim_evidence_digest(
                    &subject,
                    &claim,
                    &source_title,
                    &authority,
                    &effective_date,
                    date_evidence_type,
                    &url,
                )
            {
                return Err(
                    "The existing web evidence digest does not match its claim.".to_string()
                );
            }
            Ok(WebClaim {
                subject,
                claim,
                source_title,
                authority,
                effective_date,
                date_evidence_type,
                url,
                accessed_at: captures[9].to_string(),
                evidence_digest,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let gap_text = web_section.1.trim_end();
    let research_gaps = if gap_text == "Every required research subject was qualified." {
        Vec::new()
    } else {
        let gap_pattern = Regex::new(
            r"^- \*\*(.+?):\*\* (evidence unavailable|network unavailable) after (\d+) bounded attempt\(s\) and (\d+) fetched page\(s\)\. No claim from this subject informed the recommendation\.$",
        )
        .map_err(|error| error.to_string())?;
        gap_text
            .lines()
            .map(|line| {
                let captures = gap_pattern.captures(line).ok_or_else(|| {
                    "The existing research gap is not in OOMU’s canonical format.".to_string()
                })?;
                Ok(ResearchGap {
                    subject: decode_markdown_line(&captures[1]),
                    reason: match &captures[2] {
                        "evidence unavailable" => ResearchGapReason::EvidenceUnavailable,
                        "network unavailable" => ResearchGapReason::NetworkUnavailable,
                        _ => unreachable!("regex restricts research gap reason"),
                    },
                    attempt_count: captures[3].parse().map_err(|_| {
                        "The existing research attempt count is invalid.".to_string()
                    })?,
                    page_count: captures[4]
                        .parse()
                        .map_err(|_| "The existing research page count is invalid.".to_string())?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    };
    Ok((web_claims, research_gaps))
}

fn decode_markdown_line(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some('\\' | '*' | '_')) {
            decoded.push(characters.next().expect("peeked Markdown escape"));
        } else {
            decoded.push(character);
        }
    }
    decoded
}

fn verify_existing_decision_pack_receipt(
    request: &DecisionPackToolRequest,
    inputs: &[VerifiedInput],
    receipt: &ExistingDecisionPackReceipt,
) -> Result<(), String> {
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION
        || !is_sha256(&receipt.analysis_sha256)
        || receipt.recommendation.trim().is_empty()
        || receipt.email_summary.trim().is_empty()
        || receipt.files.len() != 4
    {
        return Err("The existing decision-pack receipt is incomplete.".to_string());
    }
    let output_directory = Path::new(&request.output_directory);
    let expected = [
        ("workbook", &request.outputs.workbook, b"PK".as_slice()),
        (
            "presentation",
            &request.outputs.presentation,
            b"PK".as_slice(),
        ),
        ("pdf", &request.outputs.pdf, b"%PDF-".as_slice()),
        (
            "sources",
            &request.outputs.sources,
            b"# Approved local inputs".as_slice(),
        ),
    ];
    let mut seen_kinds = std::collections::HashSet::new();
    for (kind, filename, magic) in expected {
        let expected_path = path_text(&output_directory.join(filename))?;
        let candidates = receipt
            .files
            .iter()
            .filter(|file| file.kind == kind && file.path == expected_path)
            .collect::<Vec<_>>();
        let [file] = candidates.as_slice() else {
            return Err(
                "The existing decision-pack receipt does not match the requested files."
                    .to_string(),
            );
        };
        if !seen_kinds.insert(kind) || file.byte_count == 0 || !is_sha256(&file.sha256) {
            return Err("The existing decision-pack receipt is invalid.".to_string());
        }
        let verified = verify_output(kind, Path::new(&file.path), Some(&file.sha256), magic)?;
        if verified.byte_count != file.byte_count {
            return Err("An existing decision-pack file changed after verification.".to_string());
        }
    }
    let sources_path = output_directory.join(&request.outputs.sources);
    let sources = fs::read_to_string(&sources_path)
        .map_err(|_| "The existing source ledger could not be reopened.".to_string())?;
    if !sources.contains(&format!(
        "Canonical analysis SHA-256: `{}`",
        receipt.analysis_sha256
    )) {
        return Err("The existing source ledger does not match its analysis receipt.".to_string());
    }
    for input in inputs {
        let expected_line = format!(
            "| `{}` | `{}` |",
            input.path.replace('|', "\\|"),
            input.sha256
        );
        if !sources.contains(&expected_line) {
            return Err(
                "An approved decision-pack input changed after the files were created.".to_string(),
            );
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn existing_output_unverified_error() -> String {
    serde_json::json!({
        "taskToolError": {
            "code": "decision_pack_existing_output_unverified",
            "message": "That folder already contains files, but OOMU couldn’t prove they are the unchanged results of this exact request. Choose a new empty folder so nothing is overwritten.",
            "context": { "changedState": false }
        }
    })
    .to_string()
}

fn verify_cross_format_semantics(
    analysis: &crate::artifacts::decision_pack::DecisionPackAnalysis,
    workbook_path: &Path,
    presentation_path: &Path,
    pdf_path: &Path,
    sources_path: &Path,
) -> Result<(), String> {
    let workbook = package_visible_text(
        &fs::read(workbook_path)
            .map_err(|_| "The staged workbook could not be reopened.".to_string())?,
        "workbook",
    )?;
    let presentation = package_visible_text(
        &fs::read(presentation_path)
            .map_err(|_| "The staged presentation could not be reopened.".to_string())?,
        "presentation",
    )?;
    let pdf_bytes =
        fs::read(pdf_path).map_err(|_| "The staged PDF could not be reopened.".to_string())?;
    let pdf = lopdf::Document::load_mem(&pdf_bytes)
        .map_err(|_| "The staged PDF could not be parsed for semantic verification.".to_string())?;
    let pages = pdf.get_pages().keys().copied().collect::<Vec<_>>();
    let pdf_text = normalized_semantic_text(
        &pdf.extract_text(&pages)
            .map_err(|_| "The staged PDF text could not be verified.".to_string())?,
    );
    let sources = normalized_source_register_text(
        &fs::read_to_string(sources_path)
            .map_err(|_| "The staged source register could not be reopened.".to_string())?,
    );
    let all_surfaces = [
        ("workbook", workbook.as_str()),
        ("presentation", presentation.as_str()),
        ("PDF", pdf_text.as_str()),
        ("source register", sources.as_str()),
    ];
    for rate in &analysis.rate_reconciliations {
        for required in [
            rate.name.clone(),
            rate.historical_rate.to_string(),
            rate.active_quote.to_string(),
        ] {
            require_semantic_value(&all_surfaces, &required)?;
        }
    }
    for margin in &analysis.margin_assessments {
        for required in [
            margin.name.clone(),
            margin.raw_estimated_cost.to_string(),
            margin.cogs_allocation.to_string(),
            margin.margin_percent.to_string(),
            margin.threshold_percent.to_string(),
        ] {
            require_semantic_value(&all_surfaces, &required)?;
        }
    }
    for exception in &analysis.exceptions {
        require_semantic_value(&all_surfaces, exception)?;
    }
    for claim in &analysis.web_claims {
        for required in [
            claim.subject.as_str(),
            claim.claim.as_str(),
            claim.source_title.as_str(),
            claim.authority.organization.as_str(),
            claim.authority.class.as_str(),
            claim.effective_date.as_str(),
            claim.date_evidence_type.as_str(),
            claim.url.as_str(),
            claim.accessed_at.as_str(),
            claim.evidence_digest.as_str(),
        ] {
            require_semantic_value(&all_surfaces, required)?;
        }
    }
    for gap in &analysis.research_gaps {
        require_semantic_value(&all_surfaces, &gap.subject)?;
        require_semantic_value(&all_surfaces, gap.reason.as_str())?;
    }
    let decision_surfaces = [
        ("workbook", workbook.as_str()),
        ("presentation", presentation.as_str()),
        ("PDF", pdf_text.as_str()),
    ];
    require_semantic_value(&decision_surfaces, &analysis.recommendation)?;
    Ok(())
}

fn package_visible_text(bytes: &[u8], kind: &str) -> Result<String, String> {
    let entries = if kind == "workbook" {
        crate::artifacts::workbooks::zip::read_zip(bytes)
    } else {
        crate::artifacts::presentations::zip::read_zip(bytes)
    }
    .map_err(|_| {
        format!("The staged {kind} package could not be read for semantic verification.")
    })?;
    let mut text = String::new();
    for (name, bytes) in entries {
        if !name.ends_with(".xml") && !name.ends_with(".rels") {
            continue;
        }
        let Ok(xml) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let mut inside_tag = false;
        for character in xml.chars() {
            match character {
                '<' => {
                    inside_tag = true;
                    text.push(' ');
                }
                '>' => {
                    inside_tag = false;
                    text.push(' ');
                }
                _ if !inside_tag => text.push(character),
                _ => {}
            }
        }
        text.push(' ');
    }
    Ok(normalized_semantic_text(&text))
}

fn normalized_semantic_text(value: &str) -> String {
    value
        .replace(',', "")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_source_register_text(value: &str) -> String {
    let line_breaks_decoded = value.replace("<br>", " ");
    let mut visible = String::with_capacity(line_breaks_decoded.len());
    let mut characters = line_breaks_decoded.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some('\\' | '*' | '_' | '|' | '#')) {
            visible.push(characters.next().expect("peeked Markdown escape"));
        } else {
            visible.push(character);
        }
    }
    normalized_semantic_text(&visible)
}

fn require_semantic_value(surfaces: &[(&str, &str)], required: &str) -> Result<(), String> {
    let required_display = normalized_semantic_text(required);
    let required = semantic_fingerprint(&required_display);
    for (kind, content) in surfaces {
        if !semantic_fingerprint(content).contains(&required) {
            return Err(format!(
                "The staged {kind} does not contain the canonical decision-pack value ‘{required_display}’. No files were published. (decision_pack_cross_format_mismatch)"
            ));
        }
    }
    Ok(())
}

fn semantic_fingerprint(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn analysis_scope_is_supported(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("reconcil") && value.contains("margin") && value.contains("exception")
}

fn read_verified_inputs(request: &DecisionPackToolRequest) -> Result<Vec<VerifiedInput>, String> {
    if request.input_bindings.len() != request.input_paths.len() {
        return Err("Decision-pack input approval bindings are incomplete.".to_string());
    }
    let mut total = 0usize;
    request
        .input_bindings
        .iter()
        .map(|binding| {
            let contents = crate::shield_gate::read_bound_approved_external_file_bounded(
                binding,
                MAX_INPUT_FILE_BYTES,
            )
            .map_err(|error| format!("{} ({})", error.message, error.code))?;
            total = total.checked_add(contents.bytes.len()).ok_or_else(|| {
                "Decision-pack input size overflowed its safety limit.".to_string()
            })?;
            if total > MAX_TOTAL_INPUT_BYTES {
                return Err(format!(
                    "Approved decision-pack inputs exceed the {} byte total limit.",
                    MAX_TOTAL_INPUT_BYTES
                ));
            }
            let content = String::from_utf8(contents.bytes.to_vec()).map_err(|_| {
                "Decision-pack source files must be UTF-8 JSON or text. No files were created."
                    .to_string()
            })?;
            Ok(VerifiedInput {
                path: contents.canonical_path.display().to_string(),
                sha256: contents.sha256,
                content: Zeroizing::new(content),
            })
        })
        .collect()
}

fn local_source_preamble(inputs: &[VerifiedInput], analysis_sha256: &str) -> String {
    let mut markdown = format!(
        "# Approved local inputs\n\nCanonical analysis SHA-256: `{analysis_sha256}`\n\n| Path | SHA-256 |\n|---|---|\n"
    );
    for input in inputs {
        markdown.push_str(&format!(
            "| `{}` | `{}` |\n",
            input.path.replace('|', "\\|"),
            input.sha256
        ));
    }
    markdown.push('\n');
    markdown
}

fn require_new_outputs<const N: usize>(paths: [&Path; N]) -> Result<(), String> {
    for path in paths {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(format!(
                    "Decision-pack output already exists at {}. Nothing was overwritten.",
                    path.display()
                ))
            }
            Err(_) => {
                return Err(format!(
                    "Decision-pack output could not be inspected at {}.",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

fn write_new_markdown(path: &Path, content: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "The approved source ledger could not be created safely.".to_string())?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| "The approved source ledger could not be saved completely.".to_string())
}

fn verify_output(
    kind: &'static str,
    path: &Path,
    expected_sha256: Option<&str>,
    expected_magic: &[u8],
) -> Result<VerifiedOutput, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("The {kind} output could not be reopened."))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "The {kind} output is not a non-empty regular file."
        ));
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| format!("The {kind} output could not be reopened safely."))?;
    let mut prefix = vec![0_u8; expected_magic.len()];
    file.read_exact(&mut prefix)
        .map_err(|_| format!("The {kind} output is truncated."))?;
    if prefix != expected_magic {
        return Err(format!(
            "The {kind} output failed its native format signature check."
        ));
    }
    let sha256 = sha256_file_hex(path)
        .map_err(|_| format!("The {kind} output could not be hashed after reopening."))?;
    if expected_sha256.is_some_and(|expected| expected != sha256) {
        return Err(format!(
            "The {kind} output hash does not match its export receipt."
        ));
    }
    Ok(VerifiedOutput {
        kind,
        path: path_text(path)?,
        sha256,
        byte_count: metadata.len(),
    })
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| "Decision-pack output path is not valid UTF-8.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::{
        decision_pack::{
            build_decision_pack, web_claim_evidence_digest, DateEvidenceType, DecisionPackAnalysis,
            MarginAssessment, RateReconciliation, SourceAuthority, SourceAuthorityClass, WebClaim,
        },
        helper::write_pdf,
        presentations::build_presentation,
        workbooks::build_workbook,
    };

    fn transaction_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-decision-pack-publication-{label}-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ))
    }

    #[test]
    fn analysis_scope_requires_all_three_decision_controls() {
        assert!(analysis_scope_is_supported(
            "Reconcile every amount and margin; identify all exceptions."
        ));
        assert!(!analysis_scope_is_supported("Summarize the files."));
    }

    #[test]
    fn existing_pack_reuse_requires_matching_receipt_hashes_and_input_evidence() {
        let root = transaction_root("existing-receipt");
        fs::create_dir_all(&root).unwrap();
        let workbook = root.join("decision.xlsx");
        let presentation = root.join("decision.pptx");
        let pdf = root.join("decision.pdf");
        let sources = root.join("sources.md");
        fs::write(&workbook, b"PK verified workbook").unwrap();
        fs::write(&presentation, b"PK verified presentation").unwrap();
        fs::write(&pdf, b"%PDF- verified document").unwrap();
        let input = VerifiedInput::test("/tmp/source|input.json", "{}");
        let analysis_sha256 = "a".repeat(64);
        fs::write(
            &sources,
            format!(
                "{}# Sources\n\nVerified.\n",
                local_source_preamble(std::slice::from_ref(&input), &analysis_sha256)
            ),
        )
        .unwrap();
        let request = DecisionPackToolRequest {
            title: "Supplier decision".to_string(),
            locale: "en-US".to_string(),
            input_paths: vec![input.path.clone()],
            research_queries: vec!["official fuel conditions".to_string()],
            research_policy: None,
            analysis_instructions: "Reconcile amounts, assess margins, and identify exceptions."
                .to_string(),
            output_directory: root.display().to_string(),
            outputs: super::super::DecisionPackOutputs {
                workbook: "decision.xlsx".to_string(),
                presentation: "decision.pptx".to_string(),
                pdf: "decision.pdf".to_string(),
                sources: "sources.md".to_string(),
            },
            input_bindings: Vec::new(),
            output_binding: None,
        };
        let file_receipt = |kind: &str, path: &Path| ExistingDecisionPackFileReceipt {
            kind: kind.to_string(),
            path: path.display().to_string(),
            sha256: sha256_file_hex(path).unwrap(),
            byte_count: fs::metadata(path).unwrap().len(),
        };
        let receipt = ExistingDecisionPackReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            analysis_sha256,
            recommendation: "Proceed after closing the listed exceptions.".to_string(),
            email_summary: "The verified supplier decision pack is ready for review.".to_string(),
            files: vec![
                file_receipt("workbook", &workbook),
                file_receipt("presentation", &presentation),
                file_receipt("pdf", &pdf),
                file_receipt("sources", &sources),
            ],
        };

        verify_existing_decision_pack_receipt(&request, &[input], &receipt).unwrap();
        fs::write(&pdf, b"%PDF- changed document").unwrap();
        assert!(verify_existing_decision_pack_receipt(&request, &[], &receipt).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_pack_without_a_saved_receipt_is_reconstructed_only_from_canonical_evidence() {
        let root = transaction_root("reconstructed-receipt");
        fs::create_dir_all(&root).unwrap();
        let inputs = vec![
            VerifiedInput::test(
                "/approved/supplier_proposals.json",
                r#"{"audit_year":2026,"quarter":"Q2","suppliers":[{"name":"Apex","historical_settled_rate":45000,"active_quote":46500,"status":"PENDING"}]}"#,
            ),
            VerifiedInput::test(
                "/approved/vendor_proposals.txt",
                "Document ID: RFP-2026-Q3-LOG\nTarget Margin Threshold: 65%\n--- VENDOR A: MATRIX SHIPPING ---\nRaw Estimated Cost: $38,000.00\nCost of Goods Sold (COGS) Allocation: $11,020.00\nGross Projected Margin: 71.0%\nCompliance Status: Fully certified.",
            ),
        ];
        let authority = SourceAuthority {
            profile_id: "usBureauTransportationStatistics".to_string(),
            organization: "U.S. Bureau of Transportation Statistics".to_string(),
            class: SourceAuthorityClass::Government,
        };
        let subject = "freight";
        let claim_text = "Official freight conditions [source date 2026-07-15] were reviewed.";
        let source_title = "Official freight update";
        let effective_date = "2026-07-15";
        let url = "https://www.bts.gov/freight-indicators";
        let claim = WebClaim {
            subject: subject.to_string(),
            claim: claim_text.to_string(),
            source_title: source_title.to_string(),
            authority: authority.clone(),
            effective_date: effective_date.to_string(),
            date_evidence_type: DateEvidenceType::PublicationDate,
            url: url.to_string(),
            accessed_at: "2026-08-05T12:00:00Z".to_string(),
            evidence_digest: web_claim_evidence_digest(
                subject,
                claim_text,
                source_title,
                &authority,
                effective_date,
                DateEvidenceType::PublicationDate,
                url,
            ),
        };
        let analysis =
            build_analysis("Supplier Decision Pack", &inputs, vec![claim], Vec::new()).unwrap();
        let analysis_sha256 =
            sha256_hex(&serde_json::to_vec(&analysis).expect("serialize analysis"));
        let artifacts = build_decision_pack(&analysis).unwrap();
        let workbook_path = root.join("decision.xlsx");
        let presentation_path = root.join("decision.pptx");
        let pdf_path = root.join("decision.pdf");
        let sources_path = root.join("sources.md");
        fs::write(
            &workbook_path,
            build_workbook(&artifacts.workbook).unwrap().bytes,
        )
        .unwrap();
        fs::write(
            &presentation_path,
            build_presentation(&artifacts.presentation).unwrap().bytes,
        )
        .unwrap();
        write_pdf(&artifacts.document, &pdf_path).unwrap();
        fs::write(
            &sources_path,
            local_source_preamble(&inputs, &analysis_sha256) + &artifacts.sources_markdown,
        )
        .unwrap();
        let request = DecisionPackToolRequest {
            title: "Supplier Decision Pack".to_string(),
            locale: "en-US".to_string(),
            input_paths: inputs.iter().map(|input| input.path.clone()).collect(),
            research_queries: vec!["official freight conditions".to_string()],
            research_policy: None,
            analysis_instructions: "Reconcile amounts, assess margins, and identify exceptions."
                .to_string(),
            output_directory: root.display().to_string(),
            outputs: super::super::DecisionPackOutputs {
                workbook: "decision.xlsx".to_string(),
                presentation: "decision.pptx".to_string(),
                pdf: "decision.pdf".to_string(),
                sources: "sources.md".to_string(),
            },
            input_bindings: Vec::new(),
            output_binding: None,
        };

        let receipt = reconstruct_existing_decision_pack_receipt(&request, &inputs).unwrap();
        assert_eq!(receipt.analysis_sha256, analysis_sha256);
        assert_eq!(receipt.files.len(), 4);
        let receipt_json = serde_json::to_value(&receipt).unwrap();
        assert_eq!(
            receipt_json["files"][0]["byteCount"].as_u64(),
            Some(receipt.files[0].byte_count)
        );
        assert!(receipt_json["files"][0].get("byte_count").is_none());
        fs::write(&presentation_path, b"PK changed presentation").unwrap();
        assert!(reconstruct_existing_decision_pack_receipt(&request, &inputs).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn production_exports_reopen_with_identical_canonical_decision_values() {
        let mut analysis = DecisionPackAnalysis {
            title: "Supplier Network Decision".to_string(),
            executive_summary: "The approved proposals were reconciled against the same canonical analysis.".to_string(),
            recommendation: "Proceed with North freight after resolving the insurance exception and confirming quote validity.".to_string(),
            rate_reconciliations: vec![RateReconciliation {
                name: "North freight".to_string(),
                historical_rate: 120.0,
                active_quote: 129.5,
                status: "Review variance".to_string(),
            }],
            margin_assessments: vec![MarginAssessment {
                name: "North freight".to_string(),
                raw_estimated_cost: 200.0,
                cogs_allocation: 155.0,
                margin_percent: 22.5,
                threshold_percent: 18.0,
                notes: "Headroom remains after the active quote.".to_string(),
            }],
            exceptions: vec![
                "Insurance coverage is not explicit in the active quote.".to_string(),
                "Source-period mismatch: /approved/mock_data/supplier_proposals.json identifies Q2 2026, while /approved/mock_data/q3_strategic_vendor_proposals.txt identifies Q3 2026. Reconcile the reporting period before award.".to_string(),
            ],
            web_claims: vec![WebClaim::test(
                "freight",
                "Official freight capacity update [source date 2026-07-15].",
                "https://www.bts.gov/freight-indicators",
            )],
            research_gaps: Vec::new(),
            email_summary: "Proceed with North freight after the named conditions are closed."
                .to_string(),
        };
        analysis.rate_reconciliations[0].name = "North|East\nFreight".to_string();
        let artifacts = build_decision_pack(&analysis).unwrap();
        let root = transaction_root("semantic-parity");
        fs::create_dir_all(&root).unwrap();
        let workbook_path = root.join("decision.xlsx");
        let presentation_path = root.join("decision.pptx");
        let pdf_path = root.join("decision.pdf");
        let sources_path = root.join("sources.md");
        fs::write(
            &workbook_path,
            build_workbook(&artifacts.workbook).unwrap().bytes,
        )
        .unwrap();
        fs::write(
            &presentation_path,
            build_presentation(&artifacts.presentation).unwrap().bytes,
        )
        .unwrap();
        write_pdf(&artifacts.document, &pdf_path).unwrap();
        fs::write(&sources_path, &artifacts.sources_markdown).unwrap();
        assert!(artifacts
            .sources_markdown
            .contains("supplier\\_proposals.json"));
        assert!(artifacts
            .sources_markdown
            .contains("North\\|East<br>Freight"));

        verify_cross_format_semantics(
            &analysis,
            &workbook_path,
            &presentation_path,
            &pdf_path,
            &sources_path,
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn staged_directory_is_published_once_with_all_files() {
        let root = transaction_root("commit");
        let final_directory = root.join("decision_pack");
        fs::create_dir_all(&final_directory).unwrap();
        let mut publication =
            DecisionPackPublication::from_owned_directory(final_directory.clone()).unwrap();
        let staging = root.join(".owned-stage");
        publication.relocate_owned_directory(&staging).unwrap();
        assert!(!final_directory.exists());
        for name in [
            "decision.xlsx",
            "decision.pptx",
            "decision.pdf",
            "sources.md",
        ] {
            fs::write(staging.join(name), format!("verified {name}")).unwrap();
        }

        publication.publish().unwrap();
        for name in [
            "decision.xlsx",
            "decision.pptx",
            "decision.pdf",
            "sources.md",
        ] {
            assert_eq!(
                fs::read_to_string(final_directory.join(name)).unwrap(),
                format!("verified {name}")
            );
        }
        assert!(!staging.exists());
        publication.commit();
        drop(publication);
        assert!(final_directory.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn failed_stage_is_removed_and_same_final_path_can_be_retried() {
        let root = transaction_root("retry");
        let final_directory = root.join("decision_pack");
        fs::create_dir_all(&final_directory).unwrap();
        let staging = root.join(".owned-stage");
        {
            let mut publication =
                DecisionPackPublication::from_owned_directory(final_directory.clone()).unwrap();
            publication.relocate_owned_directory(&staging).unwrap();
            fs::write(staging.join("decision.xlsx"), b"partial output").unwrap();
        }

        assert!(!staging.exists());
        assert!(!final_directory.exists());
        fs::create_dir(&final_directory).expect("the exact approved path remains retryable");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn publication_never_replaces_a_competing_final_directory() {
        let root = transaction_root("no-replace");
        let final_directory = root.join("decision_pack");
        fs::create_dir_all(&final_directory).unwrap();
        let staging = root.join(".owned-stage");
        let mut publication =
            DecisionPackPublication::from_owned_directory(final_directory.clone()).unwrap();
        publication.relocate_owned_directory(&staging).unwrap();
        fs::write(staging.join("decision.xlsx"), b"owned output").unwrap();
        fs::create_dir(&final_directory).unwrap();
        fs::write(final_directory.join("keep.txt"), b"unrelated data").unwrap();

        let error = publication.publish().unwrap_err();
        assert!(error.contains("appeared before publication"));
        drop(publication);
        assert!(!staging.exists());
        assert_eq!(
            fs::read(final_directory.join("keep.txt")).unwrap(),
            b"unrelated data"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn post_publication_failure_removes_only_owned_output_and_allows_retry() {
        let root = transaction_root("post-publication-retry");
        let final_directory = root.join("decision_pack");
        let unrelated_directory = root.join("unrelated");
        fs::create_dir_all(&final_directory).unwrap();
        fs::create_dir(&unrelated_directory).unwrap();
        fs::write(unrelated_directory.join("keep.txt"), b"unrelated data").unwrap();
        {
            let mut publication =
                DecisionPackPublication::from_owned_directory(final_directory.clone()).unwrap();
            let staging = root.join(".owned-stage");
            publication.relocate_owned_directory(&staging).unwrap();
            fs::write(staging.join("decision.xlsx"), b"verified output").unwrap();
            publication.publish().unwrap();
        }

        assert!(!final_directory.exists());
        assert_eq!(
            fs::read(unrelated_directory.join("keep.txt")).unwrap(),
            b"unrelated data"
        );
        fs::create_dir(&final_directory).expect("the exact approved path remains retryable");
        fs::remove_dir_all(root).unwrap();
    }
}
