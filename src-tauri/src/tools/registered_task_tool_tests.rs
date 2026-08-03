use crate::{
    agentic_loop::{
        execute_authorized_agent_action, generated_step_to_step, step_to_request, Tool,
    },
    artifacts::presentations::{
        deterministic_presentation_fixture, register_presentation_task_tool,
    },
    gemma::{generated_plan_from_text_strict, GeneratedToolDraft},
    shield_gate::{
        authorize_action, build_shield_approval_request, AuthorizedActionBoundary,
        AuthorizedActions, CommandStatus, ExecuteCommandResponse,
    },
    tools::{
        registry::{ModelProvider, NativeToolRegistry},
        task_tool_runtime,
    },
};
use serde_json::{json, Value};

#[test]
fn presentation_is_reachable_from_model_schema_through_shield_resolution() {
    let _ = register_presentation_task_tool();
    let arguments = json!({"presentation": deterministic_presentation_fixture()});
    let registry = NativeToolRegistry::default();
    let schema_payload = registry.schema_payload(ModelProvider::LocalGemmaIt);
    assert!(schema_payload["tools"].as_array().is_some_and(|tools| tools
        .iter()
        .any(|tool| tool["kind"] == "create_presentation")));
    assert!(crate::tools::registry::local_gemma_action_plan_contract()
        .pointer("/tools/create_presentation/inputSchema")
        .is_some());
    assert!(registry
        .validate_call("create_presentation", &arguments)
        .is_ok());
    let brief = json!({"brief":{
        "title":"Quarterly outlook",
        "summary":"Revenue improved while response times fell.",
        "locale":"en-US"
    }});
    let normalized_brief = registry
        .validate_call("create_presentation", &brief)
        .unwrap();
    assert!(normalized_brief.arguments.get("presentation").is_some());
    assert!(normalized_brief.arguments.get("brief").is_none());

    let generated = json!({
        "steps":[{
            "step":"Create the private presentation review.",
            "tool":{"kind":"create_presentation","presentation":arguments["presentation"]},
            "risk_level":"high"
        }],
        "exit_condition":"Return the verified private presentation review."
    });
    let draft = generated_plan_from_text_strict(generated.to_string()).unwrap();
    assert!(matches!(
        &draft.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, .. }
            if operation == "create_presentation"
    ));
    let step = generated_step_to_step(draft.steps.into_iter().next().unwrap());
    assert!(matches!(&step.tool, Tool::RegisteredTaskTool(request)
        if request.operation == "create_presentation"));
    let planned = step_to_request(&step);
    assert!(build_shield_approval_request(&planned).is_none());
    let authorized = authorize_action(planned.clone()).unwrap();
    assert!(
        matches!(&authorized, AuthorizedActions::RegisteredTaskTool(request)
        if request.operation == "create_presentation")
    );
    assert_eq!(authorized.operation_name(), "create_presentation");

    let root = std::env::temp_dir().join(format!(
        "oomu-presentation-planner-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ms_u64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence =
        crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let (resolved, resolved_request) =
        task_tool_runtime::resolve_authorized_action(&persistence, None, authorized, planned, &[])
            .unwrap();
    assert!(
        matches!(resolved, AuthorizedActions::RegisteredTaskTool(request)
        if request.operation == "create_presentation")
    );
    assert_eq!(resolved_request.kind, "create_presentation");
    assert!(resolved_request
        .content
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .is_some());
}

#[tokio::test]
async fn agentic_loop_executes_registered_task_tool_without_an_operation_branch() {
    fn validate(arguments: Value) -> Result<task_tool_runtime::TaskToolValidation, String> {
        if arguments["value"] != "ready" {
            return Err("agent_execution_fixture_invalid".to_string());
        }
        Ok(task_tool_runtime::TaskToolValidation {
            arguments,
            potentially_effectful: false,
        })
    }
    fn execute<'a>(
        _context: task_tool_runtime::TaskToolExecutionContext<'a>,
        _arguments: Value,
    ) -> task_tool_runtime::TaskToolFuture<'a> {
        Box::pin(async {
            Ok(ExecuteCommandResponse {
                operation: "agent_execution_fixture".to_string(),
                status: CommandStatus::Completed,
                message: "ready".to_string(),
                metrics: None,
                claims: vec!["CLAIM registered_task_tool_executed=true".to_string()],
                verified: true,
                model_used: None,
            })
        })
    }
    let _ = task_tool_runtime::register(task_tool_runtime::TaskToolRegistration {
        operation: "agent_execution_fixture",
        validate,
        validate_resolved: validate,
        resolve: task_tool_runtime::identity_resolver,
        execute,
        planner_context: None,
        schema: || json!({"type":"object"}),
        metadata: task_tool_runtime::TaskToolMetadata {
            description: "Test-only agent execution fixture.",
            risk_tier: task_tool_runtime::TaskToolRiskTier::ReadOnly,
            approval_tier: task_tool_runtime::TaskToolApprovalTier::Background,
            agent_error_code: "agent_execution_fixture_failed",
            agent_error_boundary: "AgentExecutionFixture",
            execution_path: "The generic registered Task-tool branch executed the fixture.",
        },
    });
    let request = task_tool_runtime::PlannedTaskToolRequest::new(
        "agent_execution_fixture",
        json!({"value":"ready"}),
    );
    let authorized = authorize_action(task_tool_runtime::requested_action(&request)).unwrap();
    let root = std::env::temp_dir().join(format!(
        "oomu-agent-tool-execution-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ms_u64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence =
        crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let identity = crate::sovereign_identity::SovereignIdentity::initialize_ephemeral();
    identity.generate_node_identity().unwrap();
    let mut execution_path = Vec::new();
    let output = execute_authorized_agent_action(
        authorized,
        None,
        &identity,
        &persistence,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &mut execution_path,
    )
    .await
    .unwrap();
    assert_eq!(output.operation, "agent_execution_fixture");
    assert!(output.verified);
    assert_eq!(
        execution_path,
        ["The generic registered Task-tool branch executed the fixture."]
    );
}
