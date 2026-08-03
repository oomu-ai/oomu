use super::{BoundWorkbookEvidence, WorkbookDateSystem, WorkbookIr};
use crate::{
    p0_contracts::{ArtifactId, ProjectId, TaskId, TaskRunId},
    p1_contracts::{
        ArtifactWorkbook, P1ContractType, P1EvidenceReference,
        WorkbookDateSystem as ContractDateSystem, WorkbookWorksheet, P1_CONTRACT_VERSION,
    },
};
use std::collections::{BTreeMap, HashSet};

pub(crate) fn artifact_workbook_contract(
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    artifact_id: &str,
    workbook: &WorkbookIr,
    bound_evidence: &[BoundWorkbookEvidence],
) -> Result<ArtifactWorkbook, String> {
    let project_id = ProjectId::parse(project_id)?;
    let task_id = TaskId::parse(task_id)?;
    let task_run_id = TaskRunId::parse(task_run_id)?;
    let artifact_id = ArtifactId::parse(artifact_id)?;
    let worksheets = workbook
        .worksheets
        .iter()
        .map(|sheet| WorkbookWorksheet {
            sheet_id: sheet.sheet_id.clone(),
            project_id: project_id.clone(),
            name: sheet.name.clone(),
            row_count: u64::from(sheet.bounds.row_count),
            column_count: u64::from(sheet.bounds.column_count),
            extensions: BTreeMap::new(),
        })
        .collect();
    let mut seen = HashSet::new();
    let evidence = bound_evidence
        .iter()
        .filter_map(|source| {
            let reference = format!("{}:{}", source.source_ref, source.evidence_ref);
            seen.insert(reference.clone()).then(|| P1EvidenceReference {
                project_id: project_id.clone(),
                evidence_class: source.evidence_class,
                reference,
                task_run_id: Some(task_run_id.clone()),
                extensions: BTreeMap::new(),
            })
        })
        .collect();
    let contract = ArtifactWorkbook {
        schema_version: P1_CONTRACT_VERSION,
        contract_type: P1ContractType::ArtifactWorkbook,
        project_id,
        task_id,
        task_run_id,
        artifact_id,
        revision: u64::from(workbook.revision),
        locale: workbook.locale.clone(),
        date_system: match workbook.date_system {
            WorkbookDateSystem::Excel1900 => ContractDateSystem::Excel1900,
            WorkbookDateSystem::Excel1904 => ContractDateSystem::Excel1904,
        },
        worksheets,
        evidence,
        extensions: BTreeMap::new(),
    };
    let encoded = serde_json::to_value(&contract).map_err(|error| error.to_string())?;
    serde_json::from_value(encoded)
        .map_err(|error| format!("Workbook artifact envelope is invalid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::deterministic_fixture;
    use crate::p0_contracts::EvidenceClass;

    #[test]
    fn artifact_workbook_envelope_round_trips_and_rejects_cross_project_sheets() {
        let project = ProjectId::new().to_string();
        let task = TaskId::new().to_string();
        let run = TaskRunId::new().to_string();
        let artifact = ArtifactId::new().to_string();
        let evidence = vec![BoundWorkbookEvidence {
            source_ref: "connector.read_verified".to_string(),
            evidence_ref: format!("task-event:{run}:7"),
            evidence_class: EvidenceClass::VerifiedPostcondition,
        }];
        let contract = artifact_workbook_contract(
            &project,
            &task,
            &run,
            &artifact,
            &deterministic_fixture().unwrap(),
            &evidence,
        )
        .unwrap();
        assert_eq!(
            contract.evidence[0].evidence_class,
            EvidenceClass::VerifiedPostcondition
        );
        let mut encoded = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            serde_json::from_value::<ArtifactWorkbook>(encoded.clone()).unwrap(),
            contract
        );
        encoded["worksheets"][0]["projectId"] =
            serde_json::Value::String(ProjectId::new().to_string());
        assert!(serde_json::from_value::<ArtifactWorkbook>(encoded).is_err());
    }
}
