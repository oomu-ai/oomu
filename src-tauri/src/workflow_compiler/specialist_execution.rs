use super::{
    hydrate_mcp_output_schemas, missing_capability_reason, missing_capability_titles,
    missing_grounded_capabilities, registered_task_capabilities, unix_time_ms,
    validate_compiler_output, validate_workflow_ir_topology, CompilerInstruction, CompilerOutput,
    ComposeWorkflowRequest, ComposeWorkflowResponse, EvaluationProtocol, FailureAction,
    VariableMapping, WorkflowCompilerError, COMPILER_VERSION,
};
use crate::workflow_ir::{WorkflowIr, WorkflowNode};

pub(super) fn compile_registered_specialist_instructions(
    workflow_ir: &WorkflowIr,
) -> Result<CompilerOutput, WorkflowCompilerError> {
    let instructions = workflow_ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::Agent(agent) => Some(CompilerInstruction {
                node_id: agent.id.clone(),
                system_prompt: agent.objective.clone(),
                input_variable_mappings: agent
                    .input_mappings
                    .iter()
                    .map(|(name, template)| VariableMapping {
                        name: name.clone(),
                        template: template.clone(),
                    })
                    .collect(),
                evaluation_protocol: EvaluationProtocol {
                    success_criteria: vec![
                        "Return one grounded result using only the mapped evidence.".to_string(),
                    ],
                    failure_action: FailureAction::Fail,
                    max_retries: 0,
                },
            }),
            _ => None,
        })
        .collect();
    let output = CompilerOutput {
        compiler_version: COMPILER_VERSION.to_string(),
        instructions,
    };
    validate_compiler_output(&output, workflow_ir)?;
    Ok(output)
}

pub(super) fn specialist_compose_response(
    mut workflow_ir: WorkflowIr,
    request: &ComposeWorkflowRequest,
    started_at: i64,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    hydrate_mcp_output_schemas(&mut workflow_ir, &request.capability_catalog);
    workflow_ir
        .validate()
        .map_err(WorkflowCompilerError::invalid_ir)?;
    validate_workflow_ir_topology(&workflow_ir)?;
    registered_task_capabilities::validate_objective_bindings(&request.prompt, &workflow_ir)?;
    let missing = missing_grounded_capabilities(&workflow_ir, &request.capability_catalog)
        .map_err(|error| WorkflowCompilerError::contract(error.message))?;
    if !missing.is_empty() {
        return Ok(ComposeWorkflowResponse {
            status: "needs_connection",
            reason: missing_capability_reason(&missing),
            workflow_ir: None,
            partial_draft: serde_json::to_value(&workflow_ir).ok(),
            missing_capabilities: missing_capability_titles(&missing),
            missing_capability_details: missing,
            composed_by: "registered_task_specialist",
            attempts: 1,
            latency_ms: unix_time_ms().saturating_sub(started_at),
        });
    }

    Ok(ComposeWorkflowResponse {
        status: "composed",
        reason: "Workflow composed from verified registered capabilities.".to_string(),
        workflow_ir: Some(workflow_ir),
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
        composed_by: "registered_task_specialist",
        attempts: 1,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}
