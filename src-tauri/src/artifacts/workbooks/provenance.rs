use super::WorkbookIr;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId},
};
use rusqlite::params;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundWorkbookEvidence {
    pub source_ref: String,
    pub evidence_ref: String,
    pub evidence_class: EvidenceClass,
}

pub(crate) fn bind_workbook_provenance(
    engine: &PersistenceEngine,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    workbook: &mut WorkbookIr,
) -> Result<Vec<BoundWorkbookEvidence>, String> {
    let events = load_task_events(engine, task_run_id)?;
    bind_from_events(project_id, task_id, task_run_id, workbook, events)
}

pub(crate) fn resolve_workbook_evidence(
    engine: &PersistenceEngine,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    workbook: &WorkbookIr,
) -> Result<Vec<BoundWorkbookEvidence>, String> {
    let mut checked = workbook.clone();
    bind_workbook_provenance(engine, project_id, task_id, task_run_id, &mut checked)
}

fn load_task_events(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<Vec<(u64, P0EventEnvelope)>, String> {
    let task_run_id = TaskRunId::parse(task_run_id)?.to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT sequence,event_json FROM task_events WHERE task_run_id=?1 ORDER BY sequence",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .map(|row| {
            let (sequence, encoded) = row.map_err(|error| error.to_string())?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| "Task evidence sequence is invalid.".to_string())?;
            let event = serde_json::from_str::<P0EventEnvelope>(&encoded)
                .map_err(|_| "Task evidence envelope is invalid.".to_string())?;
            Ok((sequence, event))
        })
        .collect();
    rows
}

pub(super) fn bind_from_events(
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    workbook: &mut WorkbookIr,
    events: Vec<(u64, P0EventEnvelope)>,
) -> Result<Vec<BoundWorkbookEvidence>, String> {
    let project = ProjectId::parse(project_id)?;
    let task = TaskId::parse(task_id)?;
    let run = TaskRunId::parse(task_run_id)?;
    let index = events
        .into_iter()
        .filter_map(|(stored_sequence, event)| {
            (event.sequence == stored_sequence
                && event.project_id == project
                && event.task_id == task
                && event.task_run_id.as_ref() == Some(&run))
            .then(|| {
                (
                    (
                        event.event_type,
                        canonical_evidence_ref(task_run_id, stored_sequence),
                    ),
                    event.evidence_class,
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut resolved = BTreeMap::new();
    for cell in workbook
        .worksheets
        .iter_mut()
        .flat_map(|sheet| &mut sheet.cells)
    {
        let mut cell_seen = HashSet::new();
        cell.provenance.retain(|source| {
            let key = (source.source_ref.clone(), source.evidence_ref.clone());
            let Some(evidence_class) = index.get(&key).copied() else {
                return false;
            };
            if !cell_seen.insert(key.clone()) {
                return false;
            }
            resolved.entry(key).or_insert(evidence_class);
            true
        });
    }
    Ok(resolved
        .into_iter()
        .map(
            |((source_ref, evidence_ref), evidence_class)| BoundWorkbookEvidence {
                source_ref,
                evidence_ref,
                evidence_class,
            },
        )
        .collect())
}

fn canonical_evidence_ref(task_run_id: &str, sequence: u64) -> String {
    format!("task-event:{task_run_id}:{sequence}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::ProvenanceReference;
    use crate::{
        artifacts::workbooks::deterministic_fixture,
        p0_contracts::{ProjectId, TaskId, TaskRunId, P0_CONTRACT_VERSION},
    };
    use serde_json::json;

    fn event(
        project: ProjectId,
        task: TaskId,
        run: TaskRunId,
        sequence: u64,
        evidence_class: EvidenceClass,
    ) -> P0EventEnvelope {
        P0EventEnvelope {
            schema_version: P0_CONTRACT_VERSION,
            event_type: "connector.read_verified".to_string(),
            project_id: project,
            task_id: task,
            task_run_id: Some(run),
            correlation_id: "correlation".to_string(),
            sequence,
            timestamp: "2026-07-11T00:00:00.000Z".to_string(),
            evidence_class,
            payload: json!({}),
        }
    }

    #[test]
    fn only_bound_task_evidence_survives_with_its_actual_class() {
        let project = ProjectId::new();
        let task = TaskId::new();
        let run = TaskRunId::new();
        let mut workbook = deterministic_fixture().unwrap();
        workbook.worksheets[0].cells[0].provenance = vec![
            ProvenanceReference {
                source_ref: "connector.read_verified".to_string(),
                evidence_ref: canonical_evidence_ref(run.as_str(), 7),
                note: None,
            },
            ProvenanceReference {
                source_ref: "forged.source".to_string(),
                evidence_ref: canonical_evidence_ref(run.as_str(), 7),
                note: None,
            },
        ];
        let bound = bind_from_events(
            project.as_str(),
            task.as_str(),
            run.as_str(),
            &mut workbook,
            vec![
                (
                    7,
                    event(
                        project.clone(),
                        task.clone(),
                        run.clone(),
                        7,
                        EvidenceClass::VerifiedPostcondition,
                    ),
                ),
                (
                    8,
                    event(
                        ProjectId::new(),
                        task.clone(),
                        run.clone(),
                        8,
                        EvidenceClass::SignedArtifact,
                    ),
                ),
            ],
        )
        .unwrap();
        assert_eq!(workbook.worksheets[0].cells[0].provenance.len(), 1);
        assert_eq!(bound.len(), 1);
        assert_eq!(
            bound[0].evidence_class,
            EvidenceClass::VerifiedPostcondition
        );
        assert_eq!(bound[0].source_ref, "connector.read_verified");
    }
}
