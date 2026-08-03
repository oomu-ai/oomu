use super::PresentationIr;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId},
};
use rusqlite::params;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundPresentationEvidence {
    pub source_ref: String,
    pub evidence_ref: String,
    pub evidence_class: EvidenceClass,
}

pub fn bind_presentation_provenance(
    engine: &PersistenceEngine,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    presentation: &mut PresentationIr,
) -> Result<Vec<BoundPresentationEvidence>, String> {
    let events = load_task_events(engine, task_run_id)?;
    bind_from_events(project_id, task_id, task_run_id, presentation, events)
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
            let (sequence, json) = row.map_err(|error| error.to_string())?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| "Task evidence sequence is invalid.".to_string())?;
            let event = serde_json::from_str(&json)
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
    presentation: &mut PresentationIr,
    events: Vec<(u64, P0EventEnvelope)>,
) -> Result<Vec<BoundPresentationEvidence>, String> {
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
                        format!("task-event:{task_run_id}:{stored_sequence}"),
                    ),
                    event.evidence_class,
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut resolved = BTreeMap::new();
    for element in presentation.slides.iter().flat_map(|slide| &slide.elements) {
        let mut seen = HashSet::new();
        for anchor in &element.provenance {
            let key = (anchor.source_ref.clone(), anchor.evidence_ref.clone());
            let evidence_class = index.get(&key).copied().ok_or_else(|| {
                format!(
                    "Presentation object {} contains evidence that is not bound to this Task run.",
                    element.object_id
                )
            })?;
            if !seen.insert(key.clone()) {
                return Err(format!(
                    "Presentation object {} contains duplicate evidence anchors.",
                    element.object_id
                ));
            }
            resolved.entry(key).or_insert(evidence_class);
        }
    }
    for citation in &presentation.citations {
        let key = (citation.source_ref.clone(), citation.evidence_ref.clone());
        let evidence_class = index.get(&key).copied().ok_or_else(|| {
            format!(
                "Citation {} contains evidence that is not bound to this Task run.",
                citation.citation_id
            )
        })?;
        resolved.entry(key).or_insert(evidence_class);
    }
    for slide in &presentation.slides {
        for source in &slide.notes.source_refs {
            if !resolved.keys().any(|(source_ref, _)| source_ref == source) {
                return Err(format!(
                    "Slide {} notes reference a source that is not bound to this Task run.",
                    slide.slide_id
                ));
            }
        }
    }
    Ok(resolved
        .into_iter()
        .map(
            |((source_ref, evidence_ref), evidence_class)| BoundPresentationEvidence {
                source_ref,
                evidence_ref,
                evidence_class,
            },
        )
        .collect())
}
