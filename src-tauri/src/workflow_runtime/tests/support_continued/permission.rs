use super::*;

#[test]
fn persists_permission_context_and_resumes_without_losing_file_references() {
    let root = std::env::temp_dir().join(format!("oomu_permission_{}", unix_time_ms()));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let compiled = permission_workflow();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(
            &workflow,
            &ir,
            &compiled.instructions.values().cloned().collect::<Vec<_>>(),
            true,
        )
        .unwrap();

    let request = RunWorkflowRequest {
        workflow_id: workflow.id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"request": "build it"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let model = FileReferenceModel {
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let paused = run_persisted_workflow(
        request,
        &persistence,
        &model,
        &NoExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(paused.instance.status, ExecutionStatus::AwaitingApproval);
    let approval = paused.approval_request.unwrap();
    assert_eq!(
        approval.message,
        "Review generated Rust code before file write"
    );

    let reloaded = persistence
        .load_execution_instance(&paused.instance.id)
        .unwrap();
    let asset_path = reloaded.memory["nodes.agent1.output"]["assetPath"]
        .as_str()
        .unwrap();
    assert!(Path::new(asset_path).is_absolute());
    assert!(Path::new(asset_path).is_file());
    assert_eq!(
        Path::new(asset_path)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("agent1.md")
    );
    assert!(!reloaded.selected_edges.is_empty());

    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id.clone(),
            approval_token: approval.approval_token.clone(),
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &model,
        &NoExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap();
    assert_eq!(completed.instance.status, ExecutionStatus::Completed);
    assert_eq!(model.calls.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(
        completed.instance.memory["nodes.agent1.output"]["assetPath"],
        asset_path
    );

    let replay = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id,
            approval_token: approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &model,
        &NoExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap_err();
    assert_eq!(replay.code, "workflow_runtime_approval_consumed");
    let _ = fs::remove_dir_all(root);
}
