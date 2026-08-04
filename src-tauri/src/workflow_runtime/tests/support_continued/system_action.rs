use super::*;

#[test]
fn test_node_timeout_triggers() {
    let mut compiled = compiled_workflow(false);
    if let WorkflowNode::Agent(agent) = &mut compiled.workflow_ir.nodes[1] {
        agent.system_timeout_ms = Some(50);
    } else {
        panic!("fixture node should be an agent");
    }
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"message": "timeout"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let error = match execute_workflow(
        &compiled,
        &request,
        &SlowModel,
        &NoExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    ) {
        Ok(_) => panic!("agent execution should time out"),
        Err(error) => error,
    };

    assert_eq!(error.code, "workflow_runtime_node_timeout");
    assert_eq!(instance.status, ExecutionStatus::Failed);
    let payload = instance.node_payloads.get("agent").unwrap();
    assert_eq!(payload.status, ExecutionStatus::Failed);
    assert_eq!(
        payload.error.as_ref().unwrap()["code"],
        json!("workflow_runtime_node_timeout")
    );
    assert!(payload.error.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .contains("Node Execution Timed Out"));
    assert!(instance.output_payload.is_none());
}

#[test]
fn output_handler_writes_envelope_to_local_directory() {
    let compiled = compiled_workflow(false);
    let directory = workflow_workspace_root().join(format!("oomu_runtime_{}", unix_time_ms()));
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::from([(
            "output".to_string(),
            OutputBinding::LocalDirectory {
                directory: directory.to_string_lossy().to_string(),
                file_name: Some("result.json".to_string()),
            },
        )]),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &NoExternalTools,
        &directory,
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();
    assert!(directory.join("result.json").is_file());
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn system_action_captures_exit_code_and_limited_stdout() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::SystemAction(SystemActionNode {
        id: "agent".to_string(),
        label: "Echo".to_string(),
        action_type: SystemActionType::Binary,
        command: "echo".to_string(),
        args: vec!["abcdef".to_string()],
        working_directory: None,
        system_timeout_ms: None,
        timeout_ms: 1_000,
        max_output_bytes: 4,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let root = std::env::temp_dir().join(format!("oomu_system_action_{}", unix_time_ms()));
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &NoExternalTools,
        &root,
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    let output = &instance.memory["nodes.agent.output"]["data"];
    let run_workspace = root.join(&instance.id);
    assert_eq!(output["exitCode"], json!(0));
    assert_eq!(output["stdout"], json!("abcd"));
    assert_eq!(output["stdoutTruncated"], json!(true));
    let expected_working_directory = run_workspace
        .canonicalize()
        .unwrap_or_else(|_| run_workspace.clone())
        .to_string_lossy()
        .to_string();
    assert_eq!(
        output["workingDirectory"],
        json!(expected_working_directory)
    );
    assert!(run_workspace.is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sandboxed_python_blocks_host_file_read_and_write() {
    let status = crate::security::sandbox::sandbox_status();
    if !status.supported {
        eprintln!("Skipping malicious sandbox regression because no sandbox engine is available.");
        return;
    }

    let root = std::env::temp_dir().join(format!("oomu_exploit_{}", unix_time_ms()));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace creates");
    let sentinel_root = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
        .join("target")
        .join(format!("oomu_sandbox_sentinel_{}", unix_time_ms()));
    fs::create_dir_all(&sentinel_root).expect("sentinel directory creates");
    let sentinel = sentinel_root.join("host_secret.txt");
    let sentinel_bytes = b"readable and writable outside the action workspace\n";
    fs::write(&sentinel, sentinel_bytes).expect("host sentinel writes outside workspace");
    let script = workspace.join("exploit.py");
    fs::write(
        &script,
        r##"
from pathlib import Path
import errno
import sys

blocked = set()
sentinel = Path(sys.argv[1])

def blocked_error(exc):
    return isinstance(exc, (PermissionError, FileNotFoundError)) or getattr(exc, "errno", None) in {
        errno.EPERM,
        errno.EACCES,
        errno.EROFS,
    }

def probe_read():
    try:
        sentinel.read_text()
        print("sentinel_read_unblocked")
    except OSError as exc:
        if blocked_error(exc):
            print(f"sentinel_read_blocked:{type(exc).__name__}")
            blocked.add("read")
        else:
            print(f"sentinel_read_unexpected:{type(exc).__name__}:{getattr(exc, 'errno', None)}")

def probe_write():
    try:
        with sentinel.open("a", encoding="utf-8") as handle:
            handle.write("# oomu exploit probe\n")
        print("sentinel_write_unblocked")
    except OSError as exc:
        if blocked_error(exc):
            print(f"sentinel_write_blocked:{type(exc).__name__}")
            blocked.add("write")
        else:
            print(f"sentinel_write_unexpected:{type(exc).__name__}:{getattr(exc, 'errno', None)}")

probe_read()
probe_write()
raise SystemExit(13 if blocked == {"read", "write"} else 1)
"##,
    )
    .expect("exploit script writes inside workspace");

    let script_path = script.to_string_lossy().to_string();
    let sentinel_path = sentinel
        .canonicalize()
        .expect("host sentinel resolves")
        .to_string_lossy()
        .to_string();
    let run_result = run_system_action(
        &SystemActionType::Python,
        &script_path,
        &[sentinel_path],
        &workspace,
        &workspace,
        30_000,
        20_000,
    );
    let sentinel_after = fs::read(&sentinel);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&sentinel_root);

    let result = run_result.expect("sandboxed python action runs");
    let engine = result
        .sandbox
        .as_ref()
        .map(|metadata| metadata.engine.as_str())
        .unwrap_or("none");

    assert!(
        result.sandbox.is_some(),
        "python system action should be sandboxed"
    );
    assert!(
        !result.timed_out,
        "sandbox security probe timed out; engine={engine} stdout={} stderr={}",
        result.stdout.text, result.stderr.text
    );
    assert_eq!(
        result.exit_code,
        Some(13),
        "engine={engine} stdout={} stderr={}",
        result.stdout.text,
        result.stderr.text
    );
    assert!(result.stdout.text.contains("sentinel_read_blocked"));
    assert!(result.stdout.text.contains("sentinel_write_blocked"));
    assert_eq!(
        sentinel_after.expect("host sentinel remains readable after sandbox run"),
        sentinel_bytes,
        "sandboxed process must not modify the host sentinel"
    );
}

#[test]
#[ignore = "runs the frontend typechecker through the local code sandbox"]
fn sandboxed_npm_typecheck_runs_in_workspace() {
    let status = crate::security::sandbox::sandbox_status();
    if !status.supported {
        eprintln!("Skipping sandboxed typecheck because no sandbox engine is available.");
        return;
    }

    let repo_root = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
        .parent()
        .expect("src-tauri has a repository parent")
        .to_path_buf();
    let sandbox_home = repo_root.join(".oomu_sandbox_home");
    let result = run_system_action(
        &SystemActionType::Binary,
        "npm",
        &["run".to_string(), "typecheck".to_string()],
        &repo_root,
        &repo_root,
        120_000,
        50_000,
    )
    .expect("sandboxed npm typecheck runs");

    assert!(
        result.sandbox.is_some(),
        "npm typecheck should run in the local code sandbox"
    );
    let exit_code = result.exit_code;
    let stdout = result.stdout.text;
    let stderr = result.stderr.text;
    let _ = fs::remove_dir_all(sandbox_home);
    assert_eq!(exit_code, Some(0), "stdout={} stderr={}", stdout, stderr);
}

#[test]
fn system_action_self_heals_non_zero_exit_with_bounded_rerun() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::SystemAction(SystemActionNode {
        id: "agent".to_string(),
        label: "Fail then heal".to_string(),
        action_type: SystemActionType::Binary,
        command: "git".to_string(),
        args: vec!["status".to_string()],
        working_directory: None,
        system_timeout_ms: None,
        timeout_ms: 5_000,
        max_output_bytes: 100,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let root = std::env::temp_dir().join(format!("oomu_self_heal_{}", unix_time_ms()));
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    execute_workflow(
        &compiled,
        &request,
        &RepairFixtureModel,
        &NoExternalTools,
        &root,
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    let output = &instance.memory["nodes.agent.output"]["data"];
    assert_eq!(output["exitCode"], json!(0));
    assert_eq!(output["stdout"], json!("healed\n"));
    assert_eq!(
        instance.memory["nodes.agent.output"]["metadata"]["selfHeal"]["attempted"],
        json!(true)
    );
    assert_eq!(
        instance.memory["nodes.agent.output"]["metadata"]["selfHeal"]["originalExitCode"]
            .as_i64()
            .is_some_and(|code| code > 0),
        true
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn system_action_unwhitelisted_command_pauses_for_approval() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::SystemAction(SystemActionNode {
        id: "agent".to_string(),
        label: "Remove files".to_string(),
        action_type: SystemActionType::Binary,
        command: "rm".to_string(),
        args: vec!["-rf".to_string(), "/tmp/oomu-never-delete".to_string()],
        working_directory: None,
        system_timeout_ms: None,
        timeout_ms: 1_000,
        max_output_bytes: 4,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let paused = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &NoExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    let approval = paused
        .approval_request
        .expect("approval request is emitted");
    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    assert_eq!(approval.node_id, "agent");
    assert_eq!(approval.context["actionType"], json!("system_action"));
    assert_eq!(approval.context["command"], json!("rm"));
    assert_eq!(
        approval.context["args"],
        json!(["-rf", "/tmp/oomu-never-delete"])
    );
}

#[test]
fn system_action_shell_metacharacters_pause_for_approval() {
    let cases = [
        ("bare ampersand", "echo ok & rm -rf /"),
        ("tab separator", "echo ok\trm -rf /"),
        ("shell expansion", "echo ${HOME}"),
    ];

    for (label, command) in cases {
        assert!(
            high_risk_action(&SystemActionType::Shell, command, &[]),
            "{label} should be classified as high-risk"
        );
        let approval =
            assert_system_action_pauses_for_approval(SystemActionType::Shell, command, vec![]);
        assert_eq!(approval.context["mode"], json!(SystemActionType::Shell));
    }
}

#[test]
fn package_runners_always_pause_for_human_approval() {
    let cases = [
        (SystemActionType::Binary, "npm", vec!["test".to_string()]),
        (SystemActionType::Binary, "npx", vec!["vite".to_string()]),
        (SystemActionType::Binary, "cargo", vec!["test".to_string()]),
        (SystemActionType::Python, "script.py", Vec::new()),
    ];

    for (action_type, command, args) in cases {
        assert!(
            high_risk_action(&action_type, command, &args),
            "{command} should require approval"
        );
        assert_system_action_pauses_for_approval(action_type, command, args);
    }
}

#[test]
fn system_action_classifier_gates_writes_and_shell_chaining_patterns() {
    assert!(!high_risk_action(
        &SystemActionType::Binary,
        "git",
        &["status".to_string()]
    ));
    assert!(high_risk_action(
        &SystemActionType::Binary,
        "git",
        &["write-tree".to_string()]
    ));
    assert!(high_risk_action(
        &SystemActionType::Binary,
        "git",
        &["status; rm -rf /".to_string()]
    ));
    assert!(high_risk_action(
        &SystemActionType::Shell,
        "echo ok && rm -rf /",
        &[]
    ));
    assert!(high_risk_action(
        &SystemActionType::Binary,
        "curl",
        &["https://example.invalid".to_string()]
    ));
}

#[test]
fn tiered_timeout_defaults_follow_node_kind_and_properties() {
    let simple_agent = AgentNode {
        id: "agent".to_string(),
        label: "Summarize".to_string(),
        objective: "Summarize the input.".to_string(),
        input_mappings: HashMap::new(),
        output_key: "summary".to_string(),
        system_timeout_ms: None,
    };
    let research_agent = AgentNode {
        objective: "Run recursive research and deep reasoning.".to_string(),
        ..simple_agent.clone()
    };
    let configured_agent = AgentNode {
        system_timeout_ms: Some(42),
        ..simple_agent.clone()
    };
    let router = RouterNode {
        id: "router".to_string(),
        label: "Classify".to_string(),
        expression: "input.kind == urgent".to_string(),
        routes: vec![
            RouterRoute {
                port: "matched".to_string(),
                condition: "urgent".to_string(),
            },
            RouterRoute {
                port: "not_matched".to_string(),
                condition: "other".to_string(),
            },
        ],
        system_timeout_ms: None,
    };
    let configured_router = RouterNode {
        system_timeout_ms: Some(crate::workflow_ir::LONG_TIMEOUT_MS + 1),
        ..router.clone()
    };
    let mcp_tool = McpToolNode {
        id: "tool".to_string(),
        label: "List".to_string(),
        server_name: "local_filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    };
    let legacy_calendar_tool = McpToolNode {
        id: "calendar".to_string(),
        label: "Read Calendar".to_string(),
        server_name: "macos_applescript".to_string(),
        tool_name: "read_system_calendar".to_string(),
        arguments: json!({"hours_ahead": 24}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(10_000),
    };
    let legacy_mail_tool = McpToolNode {
        id: "mail".to_string(),
        label: "Read Mail".to_string(),
        server_name: "macos_applescript".to_string(),
        tool_name: "read_system_emails".to_string(),
        arguments: json!({"max_messages": 5}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(10_000),
    };

    assert_eq!(
        agent_timeout_ms(&simple_agent),
        crate::workflow_ir::MEDIUM_TIMEOUT_MS
    );
    assert_eq!(
        agent_timeout_ms(&research_agent),
        crate::workflow_ir::LONG_TIMEOUT_MS
    );
    assert_eq!(agent_timeout_ms(&configured_agent), 42);
    assert_eq!(
        router_timeout_ms(&router),
        crate::workflow_ir::SHORT_TIMEOUT_MS
    );
    assert_eq!(
        router_timeout_ms(&configured_router),
        crate::workflow_ir::LONG_TIMEOUT_MS
    );
    assert_eq!(
        mcp_tool_timeout_ms(&mcp_tool),
        crate::workflow_ir::MEDIUM_TIMEOUT_MS
    );
    assert_eq!(
        mcp_tool_timeout_ms(&legacy_calendar_tool),
        SYSTEM_CALENDAR_WORKFLOW_TIMEOUT_MS
    );
    assert_eq!(
        mcp_tool_timeout_ms(&legacy_mail_tool),
        APPLE_APP_WORKFLOW_TIMEOUT_MS
    );
}

#[test]
fn native_calendar_failures_keep_their_recovery_contract() {
    let permission = WorkflowRuntimeError::calendar_read(&McpToolCallResult {
        content: vec![],
        structured_content: Some(json!({
            "backend": "eventkit",
            "code": "calendar_permission_denied",
            "message": "Calendar access is not authorized.",
            "events": []
        })),
        is_error: true,
        meta: None,
        raw: None,
    });
    assert_eq!(permission.code, "workflow_runtime_calendar_permission");
    assert_eq!(permission.message, "Calendar access is not authorized.");

    let unavailable = WorkflowRuntimeError::calendar_read(&McpToolCallResult {
        content: vec![],
        structured_content: Some(json!({
            "backend": "eventkit",
            "code": "calendar_read_failed",
            "message": "Calendar could not be read.",
            "events": []
        })),
        is_error: true,
        meta: None,
        raw: None,
    });
    assert_eq!(unavailable.code, "workflow_runtime_calendar_unavailable");
    assert_eq!(unavailable.message, "Calendar could not be read.");
}
