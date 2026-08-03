use crate::{db::PersistenceEngine, p0_contracts::EvidenceClass};
use serde_json::Value;
#[cfg(test)]
use std::cell::RefCell;
#[cfg(not(test))]
use std::sync::OnceLock;

pub(crate) type RecordTaskEvent =
    fn(&PersistenceEngine, &str, &str, EvidenceClass, Value) -> Result<(), String>;
pub(crate) type RecordTaskEventWithSequence =
    fn(&PersistenceEngine, &str, &str, EvidenceClass, Value) -> Result<u64, String>;
pub(crate) type RequireBoundTask = fn(&PersistenceEngine, &str, &str) -> Result<(), String>;
pub(crate) type RequireAgentRuntimeTask =
    fn(&PersistenceEngine, &str) -> Result<AgentRuntimeTaskBinding, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRuntimeTaskBinding {
    pub task_id: String,
    pub task_run_id: String,
    pub project_id: String,
}

#[derive(Clone, Copy)]
pub(crate) struct TaskRuntimeRegistration {
    pub record_event: RecordTaskEvent,
    pub record_event_with_sequence: RecordTaskEventWithSequence,
    pub require_bound_task: RequireBoundTask,
    pub require_agent_runtime_task: RequireAgentRuntimeTask,
}

#[cfg(not(test))]
static REGISTRATION: OnceLock<TaskRuntimeRegistration> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_REGISTRATION: RefCell<Option<TaskRuntimeRegistration>> = const { RefCell::new(None) };
}

fn same_registration(left: &TaskRuntimeRegistration, right: &TaskRuntimeRegistration) -> bool {
    std::ptr::fn_addr_eq(left.record_event, right.record_event)
        && std::ptr::fn_addr_eq(
            left.record_event_with_sequence,
            right.record_event_with_sequence,
        )
        && std::ptr::fn_addr_eq(left.require_bound_task, right.require_bound_task)
        && std::ptr::fn_addr_eq(
            left.require_agent_runtime_task,
            right.require_agent_runtime_task,
        )
}

#[cfg(not(test))]
fn registration() -> Result<TaskRuntimeRegistration, String> {
    REGISTRATION
        .get()
        .copied()
        .ok_or_else(|| "task_runtime_not_registered".to_string())
}

#[cfg(test)]
fn registration() -> Result<TaskRuntimeRegistration, String> {
    TEST_REGISTRATION.with(|slot| {
        slot.borrow()
            .as_ref()
            .copied()
            .ok_or_else(|| "task_runtime_not_registered".to_string())
    })
}

#[cfg(not(test))]
pub(crate) fn register(value: TaskRuntimeRegistration) -> Result<(), String> {
    if let Some(existing) = REGISTRATION.get() {
        return same_registration(existing, &value)
            .then_some(())
            .ok_or_else(|| "task_runtime_registration_duplicate".to_string());
    }
    match REGISTRATION.set(value) {
        Ok(()) => Ok(()),
        Err(value)
            if REGISTRATION
                .get()
                .is_some_and(|existing| same_registration(existing, &value)) =>
        {
            Ok(())
        }
        Err(_) => Err("task_runtime_registration_duplicate".to_string()),
    }
}

#[cfg(test)]
pub(crate) fn register(value: TaskRuntimeRegistration) -> Result<(), String> {
    TEST_REGISTRATION.with(|slot| {
        let mut slot = slot.borrow_mut();
        match *slot {
            Some(existing) if same_registration(&existing, &value) => Ok(()),
            Some(_) => Err("task_runtime_registration_duplicate".to_string()),
            None => {
                *slot = Some(value);
                Ok(())
            }
        }
    })
}

pub(crate) fn record_event(
    engine: &PersistenceEngine,
    task_run_id: &str,
    event_type: &str,
    evidence: EvidenceClass,
    payload: Value,
) -> Result<(), String> {
    (registration()?.record_event)(engine, task_run_id, event_type, evidence, payload)
}

pub(crate) fn record_event_with_sequence(
    engine: &PersistenceEngine,
    task_run_id: &str,
    event_type: &str,
    evidence: EvidenceClass,
    payload: Value,
) -> Result<u64, String> {
    (registration()?.record_event_with_sequence)(engine, task_run_id, event_type, evidence, payload)
}

pub(crate) fn require_bound_task(
    engine: &PersistenceEngine,
    task_run_id: &str,
    project_id: &str,
) -> Result<(), String> {
    (registration()?.require_bound_task)(engine, task_run_id, project_id)
}

pub(crate) fn require_agent_runtime_task(
    engine: &PersistenceEngine,
    execution_id: &str,
) -> Result<AgentRuntimeTaskBinding, String> {
    (registration()?.require_agent_runtime_task)(engine, execution_id)
}
