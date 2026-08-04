use super::*;
use serde::Deserialize;

const LOCAL_SEARCH_SERVER_NAME: &str = "local_search";
const SEARCH_WEB_TOOL_NAME: &str = "search_web";
const SOVEREIGN_SEARCH_MCP_SCHEMA: &str = "oomu.sovereign-mcp-search.v1";
const MAX_QUERY_CHARS: usize = 500;
const MAX_RESULTS: usize = 5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovedSearchArguments {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

pub(super) async fn execute_if_supported(
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
    approval: Option<McpToolApproval>,
    approval_scope_kind: Option<&str>,
    turn_context: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
    guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
) -> Option<Result<McpToolCallResult, String>> {
    if !is_supported_tool(server_name, tool_name) {
        return None;
    }
    Some(
        execute(
            arguments,
            approval,
            approval_scope_kind,
            turn_context,
            persistence,
            registry,
            app,
            guard,
        )
        .await,
    )
}

pub(super) fn is_supported_tool(server_name: &str, tool_name: &str) -> bool {
    server_name == LOCAL_SEARCH_SERVER_NAME && tool_name == SEARCH_WEB_TOOL_NAME
}

async fn execute(
    arguments: &Value,
    approval: Option<McpToolApproval>,
    approval_scope_kind: Option<&str>,
    turn_context: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
    guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
) -> Result<McpToolCallResult, String> {
    ensure_trusted_builtin_mcp_server(registry, app, LOCAL_SEARCH_SERVER_NAME).await?;
    let parsed = parse_arguments(arguments).map_err(|error| error.message)?;
    let turn_context = turn_context.ok_or_else(|| {
        "Public web search requires the immutable context of an accepted chat turn.".to_string()
    })?;
    guard().map_err(|error| error.message)?;
    let objective =
        accepted_user_objective(persistence, turn_context).map_err(|error| error.message)?;
    let audit_id = registry
        .consume_approved_search_authority(
            arguments,
            approval,
            approval_scope_kind,
            turn_context,
            guard,
        )
        .await
        .map_err(|error| error.message)?;

    let authorization =
        crate::sovereign_search::SovereignSearchAuthorization::approved_mcp_tool_call(
            &audit_id,
            &objective,
            &parsed.query,
        );
    let response = crate::sovereign_search::execute_sovereign_duckduckgo_search(
        crate::sovereign_search::SovereignSearchExecutionRequest::approved_mcp_tool_call(
            &parsed.query,
            parsed.max_results,
            &turn_context.session_id,
            &turn_context.turn_id,
            &turn_context.generation_token,
            authorization,
        ),
        Some(app),
        Some(persistence.clone()),
    )
    .await
    .map_err(|code| format!("Sovereign public search failed ({code})."));

    let result = response.and_then(receipt_backed_result);
    eprintln!(
        "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} completion={}",
        crate::redaction::redacted_log_text(&audit_id),
        LOCAL_SEARCH_SERVER_NAME,
        SEARCH_WEB_TOOL_NAME,
        if result.as_ref().is_ok_and(|result| !result.is_error) {
            "success"
        } else {
            "blocked_or_failed"
        },
    );
    result
}

fn parse_arguments(arguments: &Value) -> Result<ApprovedSearchArguments, McpClientError> {
    validate_tool_arguments(arguments)?;
    let parsed: ApprovedSearchArguments =
        serde_json::from_value(arguments.clone()).map_err(|_| {
            McpClientError::protocol(
                "Public web search arguments must contain only query and optional max_results."
                    .to_string(),
            )
        })?;
    let canonical_query = parsed
        .query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if parsed.query.is_empty()
        || parsed.query != canonical_query
        || parsed.query.chars().count() > MAX_QUERY_CHARS
        || parsed.query.chars().any(char::is_control)
    {
        return Err(McpClientError::protocol(
            "Public web search query must be canonical, non-empty, and at most 500 characters."
                .to_string(),
        ));
    }
    if parsed
        .max_results
        .is_some_and(|maximum| maximum == 0 || maximum > MAX_RESULTS)
    {
        return Err(McpClientError::protocol(
            "Public web search max_results must be between 1 and 5.".to_string(),
        ));
    }
    Ok(parsed)
}

fn accepted_user_objective(
    persistence: &PersistenceEngine,
    turn_context: &ChatTurnPersistenceContext,
) -> Result<String, McpClientError> {
    if let Ok(objective) =
        system_mail::accepted_user_prompt_for_turn(persistence, Some(turn_context))
    {
        return Ok(objective);
    }
    if turn_context.root_turn_id != turn_context.turn_id {
        let root = persistence
            .select_chat_turn_context(&turn_context.root_turn_id)
            .map_err(|error| {
                McpClientError::permission(format!(
                    "Public web search could not verify its originating request: {error}"
                ))
            })?
            .filter(|root| {
                root.turn_id == turn_context.root_turn_id
                    && root.root_turn_id == turn_context.root_turn_id
                    && root.session_id == turn_context.session_id
                    && root.agent_id == turn_context.agent_id
            });
        if let Some(root) = root {
            if let Ok(objective) =
                system_mail::accepted_user_prompt_for_turn(persistence, Some(&root))
            {
                return Ok(objective);
            }
        }
    }
    Err(McpClientError::permission(
        "Public web search requires a durable user request bound to this exact chat lineage."
            .to_string(),
    ))
}

impl McpClientRegistry {
    async fn consume_approved_search_authority(
        &self,
        arguments: &Value,
        approval: Option<McpToolApproval>,
        approval_scope_kind: Option<&str>,
        turn_context: &ChatTurnPersistenceContext,
        guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
    ) -> Result<String, McpClientError> {
        if !self
            .has_active_trusted_builtin_session(LOCAL_SEARCH_SERVER_NAME)
            .await
        {
            return Err(McpClientError::permission(
                "Public web search was blocked because the trusted built-in search service is unavailable."
                    .to_string(),
            ));
        }
        let approval_scope_kind = match approval_scope_kind
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .unwrap_or("once")
        {
            "once" => "once",
            "chat_session" => "chat_session",
            _ => {
                return Err(McpClientError::permission(
                    "That public search approval duration is not available.".to_string(),
                ))
            }
        };
        let verified = self
            .ensure_tool_approval(
                LOCAL_SEARCH_SERVER_NAME,
                SEARCH_WEB_TOOL_NAME,
                arguments,
                approval,
            )
            .await?;
        let prepared_turn_binding =
            verified
                .public_search_turn_binding
                .as_ref()
                .ok_or_else(|| {
                    McpClientError::permission(
                        "Public search approval is missing its exact prepared chat turn binding."
                            .to_string(),
                    )
                })?;
        if !prepared_turn_binding.matches(turn_context) {
            return Err(McpClientError::permission(
                "Public search approval does not match this chat turn.".to_string(),
            ));
        }
        if !verified
            .approval_scope_kinds
            .iter()
            .any(|allowed| allowed == approval_scope_kind)
        {
            return Err(McpClientError::permission(
                "That public search approval duration was not offered for this chat.".to_string(),
            ));
        }
        self.revalidate_verified_tool_execution(
            LOCAL_SEARCH_SERVER_NAME,
            SEARCH_WEB_TOOL_NAME,
            &verified,
        )
        .await?;
        guard()?;
        let trusted_config_binding = verified
            .session
            .trusted_internal_config_binding
            .as_deref()
            .ok_or_else(|| {
                McpClientError::permission(
                    "Public search approval requires the trusted built-in search service."
                        .to_string(),
                )
            })?;
        if verified.chat_session_approved
            && !self
                .public_search_chat_session_grant_covers(
                    turn_context,
                    trusted_config_binding,
                    &verified.tool_definition_binding,
                )
                .await
        {
            return Err(McpClientError::permission(
                "Public search approval for this chat is no longer active.".to_string(),
            ));
        }
        if approval_scope_kind == "chat_session" {
            self.grant_public_search_for_chat_session(
                turn_context,
                trusted_config_binding,
                &verified.tool_definition_binding,
            )
            .await?;
            if let Err(error) = guard() {
                self.revoke_public_search_chat_session_authority(&turn_context.session_id)
                    .await;
                return Err(error);
            }
        }
        verified.audit_id.ok_or_else(|| {
            McpClientError::permission(
                "Public web search requires an exact one-use Shield approval.".to_string(),
            )
        })
    }
}

fn receipt_backed_result(
    response: crate::sovereign_search::SovereignSearchResponse,
) -> Result<McpToolCallResult, String> {
    if response.degraded {
        let code = response
            .error_code
            .as_deref()
            .filter(|code| {
                !code.is_empty()
                    && code.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            .unwrap_or("search_unavailable");
        return Ok(McpToolCallResult {
            content: vec![serde_json::json!({
                "type": "text",
                "text": format!("Sovereign public search could not complete ({code}).")
            })],
            structured_content: Some(serde_json::json!({
                "sovereignSearchError": { "code": code }
            })),
            is_error: true,
            meta: None,
            raw: None,
        });
    }
    let receipt_digest = response.receipt_digest.as_deref().unwrap_or_default();
    let invocation_index = response.invocation_index.unwrap_or_default();
    let receipt_valid = receipt_digest.len() == 64
        && receipt_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    let context_valid =
        !crate::sovereign_search::verified_sources::from_context_json(&response.context_json)
            .is_empty();
    if response.query.trim().is_empty()
        || response.engine.trim().is_empty()
        || response.result_count == 0
        || response.results.is_empty()
        || response.result_count != response.results.len()
        || !context_valid
        || !receipt_valid
        || invocation_index == 0
    {
        return Err(
            "Sovereign public search did not produce receipt-backed public evidence. (search_unavailable)"
                .to_string(),
        );
    }

    let structured = serde_json::to_value(&response)
        .map_err(|_| "Sovereign public search result could not be encoded.".to_string())?;
    let marker = serde_json::json!({
        "schema": SOVEREIGN_SEARCH_MCP_SCHEMA,
        "verified": true,
        "receiptDigest": receipt_digest,
        "invocationIndex": invocation_index,
        "query": &response.query,
        "engine": &response.engine,
        "resultCount": response.result_count,
    });
    Ok(McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": "Native sovereign public search completed with verified evidence."
        })],
        structured_content: Some(serde_json::json!({ "sovereignSearch": structured })),
        is_error: false,
        meta: Some(serde_json::json!({ "oomuSovereignSearchReceipt": marker })),
        raw: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_context(session_id: &str, agent_id: &str) -> ChatTurnPersistenceContext {
        ChatTurnPersistenceContext {
            turn_id: format!("turn-{session_id}"),
            generation_token: format!("generation-{session_id}"),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            provider_id: "local-model".to_string(),
            model_id: "test-model".to_string(),
            parent_turn_id: None,
            root_turn_id: format!("turn-{session_id}"),
            turn_kind: "user".to_string(),
        }
    }

    async fn prepare_bound_search_approval(
        registry: &McpClientRegistry,
        arguments: Value,
        turn_context: &ChatTurnPersistenceContext,
    ) -> McpToolApprovalRequest {
        let mut prepared = registry
            .prepare_tool_approval_candidate(
                LOCAL_SEARCH_SERVER_NAME,
                SEARCH_WEB_TOOL_NAME,
                arguments,
            )
            .await
            .expect("search approval prepares")
            .expect("public network search requires approval");
        registry
            .configure_public_search_chat_session_approval(
                LOCAL_SEARCH_SERVER_NAME,
                SEARCH_WEB_TOOL_NAME,
                Some(turn_context),
                &mut prepared,
            )
            .await;
        registry
            .activate_prepared_tool_approval(prepared, false)
            .await
            .expect("bound search approval activates")
    }

    #[cfg(target_os = "macos")]
    struct NativeSearchFixture {
        registry: McpClientRegistry,
        test_root: PathBuf,
        raw_invocation: PathBuf,
    }

    #[cfg(target_os = "macos")]
    impl NativeSearchFixture {
        async fn connected() -> Self {
            let python = crate::mcp::bootstrap::resolve_system_python3_headless()
                .expect("a verified Python runtime is required for the MCP boundary test");
            let test_root = std::env::temp_dir().join(format!(
                "oomu-native-public-search-{}-{}",
                std::process::id(),
                unix_time_ms()
            ));
            std::fs::create_dir_all(&test_root).expect("test root creates");
            let script = test_root.join("search_server.py");
            let raw_invocation = test_root.join("raw-search-was-invoked");
            std::fs::write(
                &script,
                r#"import json
import pathlib
import sys

sentinel = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    identifier = message.get("id")
    if method == "initialize":
        result = {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "local_search", "version": "test"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "search_web",
            "description": "Search the public web",
            "inputSchema": {"type": "object"},
        }]}
    elif method == "tools/call":
        sentinel.write_text("raw MCP search executed", encoding="utf-8")
        result = {"content": [{"type": "text", "text": "unexpected"}], "isError": False}
    else:
        result = None
    if identifier is not None and result is not None:
        print(json.dumps({"jsonrpc": "2.0", "id": identifier, "result": result}), flush=True)
"#,
            )
            .expect("test MCP server writes");
            let config = McpServerConfig {
                name: LOCAL_SEARCH_SERVER_NAME.to_string(),
                command: python,
                args: vec![
                    script.display().to_string(),
                    raw_invocation.display().to_string(),
                ],
                env: std::collections::HashMap::new(),
                transport: McpTransportConfig::Stdio,
            };
            let registry = McpClientRegistry::default();
            assert_eq!(
                registry
                    .register_trusted_server_configs([config.clone()])
                    .await,
                1
            );
            registry
                .connect_server_with_authorization(
                    config.clone(),
                    McpSpawnAuthorization::trusted_internal(&config),
                )
                .await
                .expect("trusted test search server connects");
            Self {
                registry,
                test_root,
                raw_invocation,
            }
        }
    }

    #[cfg(target_os = "macos")]
    async fn assert_stale_search_approval_is_removed(
        registry: &McpClientRegistry,
        arguments: &Value,
        chat_turn: &ChatTurnPersistenceContext,
    ) {
        let mut prepared = registry
            .prepare_tool_approval_candidate(
                LOCAL_SEARCH_SERVER_NAME,
                SEARCH_WEB_TOOL_NAME,
                arguments.clone(),
            )
            .await
            .expect("stale approval prepares")
            .expect("search requires approval");
        registry
            .configure_public_search_chat_session_approval(
                LOCAL_SEARCH_SERVER_NAME,
                SEARCH_WEB_TOOL_NAME,
                Some(chat_turn),
                &mut prepared,
            )
            .await;
        let stale_token = prepared.request.approval_token.clone();
        assert!(registry
            .activate_prepared_tool_approval_with_postcondition(prepared, false, || {
                Err(McpClientError::permission(
                    "test chat was deleted during preparation".to_string(),
                ))
            })
            .await
            .is_err());
        assert!(!registry
            .pending_tool_approvals
            .lock()
            .await
            .contains_key(&stale_token));
    }

    #[cfg(target_os = "macos")]
    async fn assert_once_search_approval_is_exact(
        registry: &McpClientRegistry,
        arguments: &Value,
        chat_turn: &ChatTurnPersistenceContext,
    ) {
        let approval = prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        let token = McpToolApproval {
            approval_token: approval.approval_token,
        };
        let audit_id = registry
            .consume_approved_search_authority(
                arguments,
                Some(token.clone()),
                Some("once"),
                chat_turn,
                &|| Ok(()),
            )
            .await
            .expect("exact approved authority is consumed");
        assert_eq!(audit_id, approval.audit_id);
        assert!(registry
            .consume_approved_search_authority(
                arguments,
                Some(token),
                Some("once"),
                chat_turn,
                &|| Ok(()),
            )
            .await
            .is_err());
    }

    #[cfg(target_os = "macos")]
    async fn assert_session_search_approval_is_bound_and_race_safe(
        registry: &McpClientRegistry,
        arguments: &Value,
        chat_turn: &ChatTurnPersistenceContext,
    ) {
        let transferred =
            prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        assert!(registry
            .consume_approved_search_authority(
                arguments,
                Some(McpToolApproval {
                    approval_token: transferred.approval_token,
                }),
                Some("chat_session"),
                &turn_context("another-session", "agent-search"),
                &|| Ok(()),
            )
            .await
            .is_err());

        let raced = prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        let guard_checks = std::sync::atomic::AtomicUsize::new(0);
        assert!(registry
            .consume_approved_search_authority(
                arguments,
                Some(McpToolApproval {
                    approval_token: raced.approval_token,
                }),
                Some("chat_session"),
                chat_turn,
                &|| {
                    if guard_checks.fetch_add(1, Ordering::SeqCst) == 0 {
                        Ok(())
                    } else {
                        Err(McpClientError::permission(
                            "test chat was deleted during grant creation".to_string(),
                        ))
                    }
                },
            )
            .await
            .is_err());
        let active_session = registry
            .session(LOCAL_SEARCH_SERVER_NAME)
            .await
            .expect("trusted search session remains active");
        let config_binding = active_session
            .trusted_internal_config_binding
            .as_deref()
            .expect("trusted search config remains bound");
        assert!(
            !registry
                .public_search_chat_session_grant_covers(
                    chat_turn,
                    config_binding,
                    &raced.tool_definition_binding,
                )
                .await
        );
    }

    #[cfg(target_os = "macos")]
    async fn grant_and_verify_chat_session_search(
        registry: &McpClientRegistry,
        arguments: &Value,
        chat_turn: &ChatTurnPersistenceContext,
    ) -> (String, String) {
        let approval = prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        registry
            .consume_approved_search_authority(
                arguments,
                Some(McpToolApproval {
                    approval_token: approval.approval_token,
                }),
                Some("chat_session"),
                chat_turn,
                &|| Ok(()),
            )
            .await
            .expect("chat session approval is consumed exactly once");
        let active_session = registry
            .session(LOCAL_SEARCH_SERVER_NAME)
            .await
            .expect("trusted search session remains active");
        let config_binding = active_session
            .trusted_internal_config_binding
            .as_deref()
            .expect("trusted search config remains bound")
            .to_string();
        assert!(
            registry
                .public_search_chat_session_grant_covers(
                    chat_turn,
                    &config_binding,
                    &approval.tool_definition_binding,
                )
                .await
        );
        assert!(
            !registry
                .public_search_chat_session_grant_covers(
                    &turn_context("another-session", "agent-search"),
                    &config_binding,
                    &approval.tool_definition_binding,
                )
                .await
        );
        assert!(
            !registry
                .public_search_chat_session_grant_covers(
                    &turn_context("session-search", "another-agent"),
                    &config_binding,
                    &approval.tool_definition_binding,
                )
                .await
        );
        (config_binding, approval.tool_definition_binding)
    }

    #[cfg(target_os = "macos")]
    async fn assert_granted_session_uses_fresh_exact_token(
        registry: &McpClientRegistry,
        chat_turn: &ChatTurnPersistenceContext,
    ) {
        let arguments = serde_json::json!({
            "query": "Writing AI Prompts for Dummies publication date",
            "max_results": 5
        });
        let transferred =
            prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        assert!(transferred.chat_session_approved);
        assert!(registry
            .consume_approved_search_authority(
                &arguments,
                Some(McpToolApproval {
                    approval_token: transferred.approval_token,
                }),
                Some("once"),
                &turn_context("session-search", "another-agent"),
                &|| Ok(()),
            )
            .await
            .is_err());
        let approval = prepare_bound_search_approval(registry, arguments.clone(), chat_turn).await;
        assert!(approval.chat_session_approved);
        registry
            .consume_approved_search_authority(
                &arguments,
                Some(McpToolApproval {
                    approval_token: approval.approval_token,
                }),
                Some("once"),
                chat_turn,
                &|| Ok(()),
            )
            .await
            .expect("next exact query consumes its own token");
    }

    fn response() -> crate::sovereign_search::SovereignSearchResponse {
        crate::sovereign_search::SovereignSearchResponse {
            query: "Writing AI Prompts for Dummies latest edition".to_string(),
            engine: "duckduckgo_lite_static".to_string(),
            result_count: 1,
            results: vec![crate::sovereign_search::SovereignSearchResult {
                title: "Writing AI Prompts For Dummies".to_string(),
                url: "https://www.wiley.com/example".to_string(),
                snippet: "1st Edition".to_string(),
            }],
            context_json: r#"{"accessedAtUtc":"2026-08-01T20:00:00Z","pages":[{"url":"https://www.wiley.com/example"}]}"#.to_string(),
            accessed_at_utc: "2026-08-01T20:00:00Z".to_string(),
            retrieval_elapsed_ms: 10,
            dom_page_count: 1,
            headless_fallback_count: 0,
            degraded: false,
            receipt_digest: Some("a".repeat(64)),
            invocation_index: Some(1),
            error_code: None,
            error: None,
            security: crate::sovereign_search::SovereignSearchSecurity {
                api_key_required: false,
                cookies_enabled: false,
                browser_automation_enabled: true,
                visible_browser_opened: false,
                proxy_environment_enabled: false,
                endpoint_allowlist: vec!["lite.duckduckgo.com".to_string()],
            },
        }
    }

    #[test]
    fn intercepts_only_the_exact_builtin_search_tool() {
        assert!(is_supported_tool("local_search", "search_web"));
        assert!(!is_supported_tool(" LOCAL_SEARCH ", " SEARCH_WEB "));
        assert!(!is_supported_tool("LOCAL_SEARCH", "SEARCH_WEB"));
        assert!(!is_supported_tool("remote_search", "search_web"));
        assert!(!is_supported_tool("local_search", "read_file"));
    }

    #[test]
    fn arguments_are_bounded_and_deny_unknown_fields() {
        let parsed = parse_arguments(&serde_json::json!({
            "query": "Writing AI Prompts for Dummies latest edition",
            "max_results": 5
        }))
        .unwrap();
        assert_eq!(
            parsed.query,
            "Writing AI Prompts for Dummies latest edition"
        );
        assert_eq!(parsed.max_results, Some(5));
        assert!(parse_arguments(&serde_json::json!({
            "query": "public facts",
            "private_context": "secret"
        }))
        .is_err());
        assert!(parse_arguments(&serde_json::json!({
            "query": "public facts",
            "max_results": 6
        }))
        .is_err());
        for noncanonical in [
            " public facts",
            "public  facts",
            "public facts ",
            "public\nfacts",
        ] {
            assert!(parse_arguments(&serde_json::json!({
                "query": noncanonical
            }))
            .is_err());
        }
    }

    #[test]
    fn result_requires_two_matching_native_authorship_surfaces() {
        let result = receipt_backed_result(response()).unwrap();
        let structured = result.structured_content.unwrap();
        let search = &structured["sovereignSearch"];
        let marker = &result.meta.unwrap()["oomuSovereignSearchReceipt"];
        assert_eq!(marker["schema"], SOVEREIGN_SEARCH_MCP_SCHEMA);
        assert_eq!(marker["verified"], true);
        for key in [
            "receiptDigest",
            "invocationIndex",
            "query",
            "engine",
            "resultCount",
        ] {
            assert_eq!(marker[key], search[key]);
        }
        assert!(result.raw.is_none());
    }

    #[test]
    fn result_fails_closed_without_positive_receipt_backed_context() {
        fn no_receipt(response: &mut crate::sovereign_search::SovereignSearchResponse) {
            response.receipt_digest = None;
        }
        fn zero_invocation(response: &mut crate::sovereign_search::SovereignSearchResponse) {
            response.invocation_index = Some(0);
        }
        fn no_context(response: &mut crate::sovereign_search::SovereignSearchResponse) {
            response.context_json = "[]".to_string();
        }
        fn no_results(response: &mut crate::sovereign_search::SovereignSearchResponse) {
            response.result_count = 0;
        }
        let mutations: [fn(&mut crate::sovereign_search::SovereignSearchResponse); 4] =
            [no_receipt, zero_invocation, no_context, no_results];
        for mutate in mutations {
            let mut invalid = response();
            mutate(&mut invalid);
            assert!(receipt_backed_result(invalid)
                .unwrap_err()
                .ends_with("(search_unavailable)"));
        }

        let mut unauthorized = response();
        unauthorized.degraded = true;
        unauthorized.error_code = Some("search_not_authorized".to_string());
        let unauthorized = receipt_backed_result(unauthorized).unwrap();
        assert!(unauthorized.is_error);
        assert!(unauthorized.meta.is_none());
        assert!(unauthorized.raw.is_none());
        assert_eq!(
            unauthorized.structured_content.unwrap()["sovereignSearchError"]["code"],
            "search_not_authorized"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn consumes_exact_one_use_approval_without_invoking_raw_search_tool() {
        let fixture = NativeSearchFixture::connected().await;
        let arguments = serde_json::json!({
            "query": "Writing AI Prompts for Dummies latest edition",
            "max_results": 5
        });
        let chat_turn = turn_context("session-search", "agent-search");
        assert_stale_search_approval_is_removed(&fixture.registry, &arguments, &chat_turn).await;
        assert_once_search_approval_is_exact(&fixture.registry, &arguments, &chat_turn).await;
        assert_session_search_approval_is_bound_and_race_safe(
            &fixture.registry,
            &arguments,
            &chat_turn,
        )
        .await;
        let (trusted_config_binding, tool_definition_binding) =
            grant_and_verify_chat_session_search(&fixture.registry, &arguments, &chat_turn).await;
        assert_granted_session_uses_fresh_exact_token(&fixture.registry, &chat_turn).await;
        assert!(!fixture.raw_invocation.exists());
        fixture
            .registry
            .shutdown_all()
            .await
            .expect("test server shuts down");
        assert!(
            !fixture
                .registry
                .public_search_chat_session_grant_covers(
                    &chat_turn,
                    &trusted_config_binding,
                    &tool_definition_binding,
                )
                .await
        );
        let _ = std::fs::remove_dir_all(fixture.test_root);
    }

    #[tokio::test]
    async fn revoked_chat_session_loses_grants_and_pending_search_tokens() {
        let registry = McpClientRegistry::default();
        let chat_turn = turn_context("session-revoked", "agent-search");
        registry
            .grant_public_search_for_chat_session(&chat_turn, "config", "tool")
            .await
            .expect("test grant creates");
        registry.pending_tool_approvals.lock().await.insert(
            "pending-search".to_string(),
            PendingMcpToolApproval {
                request: McpToolApprovalRequest {
                    approval_token: "pending-search".to_string(),
                    server_name: LOCAL_SEARCH_SERVER_NAME.to_string(),
                    tool_name: SEARCH_WEB_TOOL_NAME.to_string(),
                    arguments: serde_json::json!({}),
                    message: String::new(),
                    capability_risk_tier: String::new(),
                    capability_reason: String::new(),
                    expires_at_ms: u64::MAX,
                    argument_summary: String::new(),
                    sensitive_fields: vec![],
                    canonical_origin: None,
                    transport: "local".to_string(),
                    resolved_destination_class: None,
                    destination_binding: None,
                    server_identity_binding: None,
                    certificate_binding: None,
                    tool_definition_binding: "tool".to_string(),
                    audit_id: "audit".to_string(),
                    response_byte_limit: 1,
                    native_shield_approved: false,
                    chat_session_approved: true,
                    approval_scope_kinds: vec!["once".to_string(), "chat_session".to_string()],
                },
                arguments_binding: argument_binding(&serde_json::json!({})),
                session: None,
                public_search_turn_binding: Some(
                    PublicSearchApprovalTurnBinding::from_turn_context(&chat_turn),
                ),
            },
        );

        assert_eq!(
            registry
                .revoke_public_search_chat_session_authority(&chat_turn.session_id)
                .await,
            2
        );
        assert!(
            !registry
                .public_search_chat_session_grant_covers(&chat_turn, "config", "tool")
                .await
        );
        assert!(registry.pending_tool_approvals.lock().await.is_empty());
    }
}
