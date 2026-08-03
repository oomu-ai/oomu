use super::{BoundPresentationEvidence, PresentationAspectRatio, PresentationIr};
use crate::{
    p0_contracts::{ArtifactId, ProjectId, TaskId, TaskRunId},
    p1_contracts::{
        ArtifactPresentation, P1ContractType, P1EvidenceReference,
        PresentationAspectRatio as ContractAspectRatio, PresentationSlide, P1_CONTRACT_VERSION,
    },
};
use std::collections::{BTreeMap, HashSet};

pub fn artifact_presentation_contract(
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    artifact_id: &str,
    presentation: &PresentationIr,
    bound_evidence: &[BoundPresentationEvidence],
) -> Result<ArtifactPresentation, String> {
    let project_id = ProjectId::parse(project_id)?;
    let task_id = TaskId::parse(task_id)?;
    let task_run_id = TaskRunId::parse(task_run_id)?;
    let artifact_id = ArtifactId::parse(artifact_id)?;
    let slides = presentation
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| PresentationSlide {
            slide_id: slide.slide_id.clone(),
            project_id: project_id.clone(),
            position: index as u64,
            layout_id: slide.layout_id.clone(),
            extensions: BTreeMap::new(),
        })
        .collect();
    let mut seen = HashSet::new();
    let evidence = bound_evidence
        .iter()
        .filter_map(|item| {
            let reference = format!("{}:{}", item.source_ref, item.evidence_ref);
            seen.insert(reference.clone()).then(|| P1EvidenceReference {
                project_id: project_id.clone(),
                evidence_class: item.evidence_class,
                reference,
                task_run_id: Some(task_run_id.clone()),
                extensions: BTreeMap::new(),
            })
        })
        .collect();
    let contract = ArtifactPresentation {
        schema_version: P1_CONTRACT_VERSION,
        contract_type: P1ContractType::ArtifactPresentation,
        project_id,
        task_id,
        task_run_id,
        artifact_id,
        revision: u64::from(presentation.revision),
        aspect_ratio: match presentation.aspect_ratio {
            PresentationAspectRatio::Widescreen => ContractAspectRatio::Widescreen,
            PresentationAspectRatio::Standard => ContractAspectRatio::Standard,
        },
        slides,
        evidence,
        extensions: BTreeMap::new(),
    };
    let encoded = serde_json::to_value(&contract).map_err(|error| error.to_string())?;
    serde_json::from_value(encoded)
        .map_err(|error| format!("Presentation artifact envelope is invalid: {error}"))
}
