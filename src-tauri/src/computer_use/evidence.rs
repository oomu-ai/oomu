use super::{
    contracts::{DesktopObservation, DesktopOutcomeReceipt},
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    manager::AppControlManager,
    state::invalid_request,
};
use crate::p0_contracts::TaskRunId;

impl AppControlManager {
    pub fn receipt(
        &self,
        receipt_id: &str,
        task_run_id: &str,
    ) -> AppControlResult<Option<DesktopOutcomeReceipt>> {
        TaskRunId::parse(task_run_id).map_err(invalid_request)?;
        let state = self.lock()?;
        Ok(state
            .receipts
            .get(receipt_id)
            .filter(|receipt| receipt.task_run_id == task_run_id)
            .cloned())
    }

    pub(super) fn record_task_event(
        &self,
        task_run_id: &str,
        event_type: &str,
        evidence: crate::p0_contracts::EvidenceClass,
        payload: serde_json::Value,
    ) -> AppControlResult<()> {
        let Some(engine) = &self.evidence_engine else {
            return Ok(());
        };
        crate::tools::task_runtime::record_event(engine, task_run_id, event_type, evidence, payload)
            .map_err(|_| {
                AppControlError::new(
                    AppControlErrorCode::DriverFailure,
                    "App control could not record its Task evidence.",
                )
            })
    }

    pub(super) fn record_observation(
        &self,
        observation: &DesktopObservation,
        phase: &str,
    ) -> AppControlResult<()> {
        self.record_task_event(
            &observation.task_run_id,
            "app_control.observation",
            crate::p0_contracts::EvidenceClass::ObservedResult,
            serde_json::json!({
                "phase": phase,
                "observation": observation,
            }),
        )
    }
}
