use super::*;
use crate::{
    browser_automation::BrowserAutomationManager, db::PersistenceEngine, gemma::GemmaService,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

pub(crate) async fn execute(
    proposal: ChildProposal,
    project_id: String,
    task_run_id: String,
    app: tauri::AppHandle,
    persistence: PersistenceEngine,
    browser: BrowserAutomationManager,
    gemma: GemmaService,
    cancel: Arc<AtomicBool>,
) -> Result<ChildResult, String> {
    let timeout = std::time::Duration::from_millis(proposal.budget.timeout_ms);
    let operation = async move {
        let started = Instant::now();
        let mut materials = Vec::new();
        for source in &proposal.sources {
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            materials.push(
                sources::read(
                    source,
                    &project_id,
                    &task_run_id,
                    &app,
                    &persistence,
                    &browser,
                )
                .await?,
            );
        }
        let mut grounding = String::new();
        for (index, item) in materials.iter().enumerate() {
            grounding.push_str(&format!(
                "\nSOURCE {} [{}]\n{}\n",
                index + 1,
                item.evidence.source_ref,
                item.content
            ));
        }
        let input_tokens = grounding.len().div_ceil(4);
        if input_tokens > proposal.budget.max_input_tokens {
            return Err("input_budget_exceeded".into());
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let goal = proposal.goal.clone();
        let service = gemma.clone();
        let inference = tauri::async_runtime::spawn_blocking(move || {
            grounded_inference_sync(&service, &goal, &grounding).map_err(|e| e.message)
        })
        .await
        .map_err(|e| e.to_string())??;
        let output = inference.text.trim().to_string();
        let output_tokens = output.len().div_ceil(4);
        if output.is_empty() {
            return Err("empty_model_result".into());
        }
        if output_tokens > proposal.budget.max_output_tokens
            || output.len() > proposal.budget.max_response_bytes
        {
            return Err("output_budget_exceeded".into());
        }
        let complete = !cancel.load(Ordering::SeqCst);
        let refs = materials
            .iter()
            .map(|v| v.evidence.source_ref.clone())
            .collect();
        Ok(ChildResult {
            findings: vec![Finding {
                statement: output,
                source_refs: refs,
                confidence: "model_assertion_grounded_in_observed_sources".into(),
            }],
            sources: materials.into_iter().map(|v| v.evidence).collect(),
            uncertainties: Vec::new(),
            limitations: vec![
                "This child had read-only authority; it did not execute or verify mutations."
                    .into(),
            ],
            complete,
            actual_model_route: inference.model_path,
            elapsed_ms: started.elapsed().as_millis() as u64,
            input_tokens_estimate: input_tokens,
            output_tokens_estimate: output_tokens,
        })
    };
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| "child_timeout".to_string())?
}

pub(crate) fn grounded_inference_sync(
    gemma: &GemmaService,
    goal: &str,
    grounding: &str,
) -> Result<crate::gemma::InferResponse, crate::gemma::GemmaError> {
    gemma.summarize_grounded_text_sync(goal, grounding)
}

pub(crate) fn synthesize(children: &[ChildRunView]) -> DelegationSynthesis {
    let mut findings = Vec::new();
    let mut uncertainties = Vec::new();
    let mut incomplete = Vec::new();
    for child in children {
        match &child.result {
            Some(result) if result.complete => {
                findings.extend(result.findings.clone());
                uncertainties.extend(result.uncertainties.clone());
            }
            _ => incomplete.push(child.child_run_id.clone()),
        }
    }
    DelegationSynthesis {
        findings,
        uncertainties,
        incomplete_child_run_ids: incomplete.clone(),
        ready_for_parent_synthesis: incomplete.is_empty(),
    }
}
