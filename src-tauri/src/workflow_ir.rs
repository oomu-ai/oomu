use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

pub(crate) mod review;

pub const WORKFLOW_IR_SCHEMA_VERSION: &str = "1.0.0";
pub const WORKFLOW_COMPILER_MODEL: &str = "gemma-4-e4b-qat";
pub const LEGACY_WORKFLOW_COMPILER_MODEL: &str = "gemma-4-e2b-qat";
pub const SHORT_TIMEOUT_MS: u64 = 5_000;
pub const MEDIUM_TIMEOUT_MS: u64 = 30_000;
pub const LONG_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkflowBlueprint {
    pub workflow_id: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub visual_state: Value,
    #[serde(default)]
    pub workflow_ir: Option<WorkflowIr>,
    pub compilation_status: BlueprintCompilationStatus,
    #[serde(default)]
    pub compilation_error: Option<String>,
    pub is_active: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub compiled_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum BlueprintCompilationStatus {
    Draft,
    Compiling,
    Compiled,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompiledInstruction {
    pub id: String,
    pub workflow_id: String,
    pub workflow_version: u32,
    pub node_id: String,
    pub node_kind: WorkflowNodeKind,
    pub system_prompt: String,
    pub input_variable_mappings: HashMap<String, String>,
    pub evaluation_protocol: Value,
    pub compiler_model: String,
    pub compiler_version: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct CompiledWorkflow {
    pub workflow_ir: WorkflowIr,
    pub instructions: HashMap<String, CompiledInstruction>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeKind {
    Input,
    Agent,
    Router,
    Conditional,
    Loop,
    Permission,
    McpTool,
    SystemAction,
    Output,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ExecutionStatus {
    Pending,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NodeExecutionPayload {
    pub status: ExecutionStatus,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExecutionInstance {
    pub id: String,
    pub workflow_id: String,
    pub workflow_version: u32,
    pub status: ExecutionStatus,
    #[serde(default)]
    pub active_node_id: Option<String>,
    pub input_payload: Value,
    #[serde(default)]
    pub output_payload: Option<Value>,
    #[serde(default)]
    pub node_payloads: HashMap<String, NodeExecutionPayload>,
    #[serde(default)]
    pub memory: HashMap<String, Value>,
    #[serde(default)]
    pub selected_edges: HashSet<String>,
    #[serde(default)]
    pub pause_context: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub execution_latency_ms: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    pub created_at_ms: i64,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkflowIr {
    pub schema_version: String,
    pub workflow_id: String,
    pub workflow_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub compiler: CompilerTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompilerTarget {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowNode {
    Input(InputNode),
    Agent(AgentNode),
    Router(RouterNode),
    Conditional(ConditionalNode),
    Loop(LoopNode),
    Permission(PermissionNode),
    McpTool(McpToolNode),
    SystemAction(SystemActionNode),
    Output(OutputNode),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InputNode {
    pub id: String,
    pub label: String,
    pub output_key: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentNode {
    pub id: String,
    pub label: String,
    pub objective: String,
    #[serde(default)]
    pub input_mappings: HashMap<String, String>,
    pub output_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouterNode {
    pub id: String,
    pub label: String,
    pub expression: String,
    pub routes: Vec<RouterRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouterRoute {
    pub port: String,
    pub condition: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConditionalNode {
    pub id: String,
    pub label: String,
    pub condition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_mapping: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoopNode {
    pub id: String,
    pub label: String,
    pub items_mapping: String,
    #[serde(default = "default_loop_item_variable")]
    pub item_variable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    FileRead,
    FileWrite,
    Network,
    Process,
    McpTool,
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDeniedBehavior {
    Fail,
    Branch,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PermissionNode {
    pub id: String,
    pub label: String,
    pub permission: PermissionKind,
    pub reason: String,
    pub on_denied: PermissionDeniedBehavior,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct McpToolNode {
    pub id: String,
    pub label: String,
    pub server_name: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemActionType {
    Shell,
    Python,
    Binary,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SystemActionNode {
    pub id: String,
    pub label: String,
    pub action_type: SystemActionType,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_timeout_ms: Option<u64>,
    #[serde(default = "default_system_action_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_system_action_max_output_bytes")]
    pub max_output_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCompletionKind {
    #[default]
    Result,
    EmptyCollection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutputNode {
    pub id: String,
    pub label: String,
    pub input_mapping: String,
    pub output_schema: Value,
    #[serde(default)]
    pub completion_kind: WorkflowCompletionKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkflowEdge {
    pub id: String,
    pub source_node_id: String,
    pub source_port: String,
    pub target_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_port: Option<String>,
}

impl WorkflowNode {
    pub fn id(&self) -> &str {
        match self {
            Self::Input(node) => &node.id,
            Self::Agent(node) => &node.id,
            Self::Router(node) => &node.id,
            Self::Conditional(node) => &node.id,
            Self::Loop(node) => &node.id,
            Self::Permission(node) => &node.id,
            Self::McpTool(node) => &node.id,
            Self::SystemAction(node) => &node.id,
            Self::Output(node) => &node.id,
        }
    }

    pub fn kind(&self) -> WorkflowNodeKind {
        match self {
            Self::Input(_) => WorkflowNodeKind::Input,
            Self::Agent(_) => WorkflowNodeKind::Agent,
            Self::Router(_) => WorkflowNodeKind::Router,
            Self::Conditional(_) => WorkflowNodeKind::Conditional,
            Self::Loop(_) => WorkflowNodeKind::Loop,
            Self::Permission(_) => WorkflowNodeKind::Permission,
            Self::McpTool(_) => WorkflowNodeKind::McpTool,
            Self::SystemAction(_) => WorkflowNodeKind::SystemAction,
            Self::Output(_) => WorkflowNodeKind::Output,
        }
    }
}

impl WorkflowIr {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        validate_workflow_header(self, &mut errors);

        let mut node_ids = HashSet::new();
        for node in &self.nodes {
            if node.id().trim().is_empty() {
                errors.push("node ids must not be empty".to_string());
            } else if !node_ids.insert(node.id()) {
                errors.push(format!("duplicate node id: {}", node.id()));
            }
        }
        let node_by_id = self
            .nodes
            .iter()
            .map(|node| (node.id(), node))
            .collect::<HashMap<_, _>>();

        let mut edge_ids = HashSet::new();
        let mut edge_signatures = HashSet::new();
        let mut incoming: HashMap<&str, usize> = HashMap::new();
        let mut outgoing: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
        for edge in &self.edges {
            if !edge_ids.insert(edge.id.as_str()) {
                errors.push(format!("duplicate edge id: {}", edge.id));
            }
            let signature = (
                edge.source_node_id.as_str(),
                edge.source_port.as_str(),
                edge.target_node_id.as_str(),
                edge.target_port.as_deref(),
            );
            if !edge_signatures.insert(signature) {
                errors.push(format!("duplicate edge connection: {}", edge.id));
            }
            if edge.source_node_id == edge.target_node_id {
                errors.push(format!("self edge is not allowed: {}", edge.id));
            }
            if !node_by_id.contains_key(edge.source_node_id.as_str()) {
                errors.push(format!("edge {} has an unknown source node", edge.id));
            }
            if !node_by_id.contains_key(edge.target_node_id.as_str()) {
                errors.push(format!("edge {} has an unknown target node", edge.id));
            }
            *incoming.entry(&edge.target_node_id).or_default() += 1;
            outgoing.entry(&edge.source_node_id).or_default().push(edge);
        }

        let input_ids = self
            .nodes
            .iter()
            .filter_map(|node| matches!(node, WorkflowNode::Input(_)).then_some(node.id()))
            .collect::<Vec<_>>();
        let output_ids = self
            .nodes
            .iter()
            .filter_map(|node| matches!(node, WorkflowNode::Output(_)).then_some(node.id()))
            .collect::<Vec<_>>();
        if input_ids.is_empty() {
            errors.push("workflow must contain at least one input node".to_string());
        }
        if output_ids.is_empty() {
            errors.push("workflow must contain at least one output node".to_string());
        }

        for node in &self.nodes {
            let in_count = incoming.get(node.id()).copied().unwrap_or_default();
            let node_edges = outgoing.get(node.id()).cloned().unwrap_or_default();
            match node {
                WorkflowNode::Input(_) => {
                    if in_count != 0 {
                        errors.push(format!(
                            "input node {} cannot have incoming edges",
                            node.id()
                        ));
                    }
                    require_single_port(node.id(), &node_edges, "out", &mut errors);
                }
                WorkflowNode::Agent(agent) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_single_port(node.id(), &node_edges, "out", &mut errors);
                    require_system_timeout(node.id(), agent.system_timeout_ms, &mut errors);
                }
                WorkflowNode::McpTool(mcp_tool) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_single_port(node.id(), &node_edges, "out", &mut errors);
                    require_system_timeout(node.id(), mcp_tool.system_timeout_ms, &mut errors);
                    if mcp_tool.server_name.trim().is_empty()
                        || mcp_tool.tool_name.trim().is_empty()
                    {
                        errors.push(format!(
                            "mcp tool node {} requires serverName and toolName",
                            node.id()
                        ));
                    }
                }
                WorkflowNode::SystemAction(system_action) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_single_port(node.id(), &node_edges, "out", &mut errors);
                    require_system_timeout(node.id(), system_action.system_timeout_ms, &mut errors);
                    if system_action.command.trim().is_empty() {
                        errors.push(format!(
                            "system action node {} requires a command",
                            node.id()
                        ));
                    }
                    if system_action.timeout_ms == 0 {
                        errors.push(format!(
                            "system action node {} requires timeoutMs greater than zero",
                            node.id()
                        ));
                    }
                    if system_action.max_output_bytes == 0 {
                        errors.push(format!(
                            "system action node {} requires maxOutputBytes greater than zero",
                            node.id()
                        ));
                    }
                }
                WorkflowNode::Router(router) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_system_timeout(node.id(), router.system_timeout_ms, &mut errors);
                    if router.routes.len() < 2 {
                        errors.push(format!(
                            "router {} must define at least two routes",
                            node.id()
                        ));
                    }
                    let route_ports = router
                        .routes
                        .iter()
                        .map(|route| route.port.as_str())
                        .collect::<HashSet<_>>();
                    if route_ports.len() != router.routes.len() {
                        errors.push(format!("router {} has duplicate route ports", node.id()));
                    }
                    let edge_ports = node_edges
                        .iter()
                        .map(|edge| edge.source_port.as_str())
                        .collect::<HashSet<_>>();
                    if route_ports != edge_ports || node_edges.len() != route_ports.len() {
                        errors.push(format!(
                            "router {} must have exactly one outgoing edge per route",
                            node.id()
                        ));
                    }
                }
                WorkflowNode::Conditional(conditional) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_system_timeout(node.id(), conditional.system_timeout_ms, &mut errors);
                    if conditional.condition.trim().is_empty() {
                        errors.push(format!(
                            "conditional node {} requires a condition",
                            node.id()
                        ));
                    }
                    require_exact_ports(
                        node.id(),
                        &node_edges,
                        &["true", "false"],
                        "conditional nodes require one true edge and one false edge",
                        &mut errors,
                    );
                }
                WorkflowNode::Loop(loop_node) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    require_system_timeout(node.id(), loop_node.system_timeout_ms, &mut errors);
                    if loop_node.items_mapping.trim().is_empty() {
                        errors.push(format!("loop node {} requires an itemsMapping", node.id()));
                    }
                    if loop_node.item_variable.trim().is_empty() {
                        errors.push(format!("loop node {} requires an itemVariable", node.id()));
                    }
                    require_exact_ports(
                        node.id(),
                        &node_edges,
                        &["item", "done"],
                        "loop nodes require one item edge and one done edge",
                        &mut errors,
                    );
                }
                WorkflowNode::Permission(permission) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    let ports = node_edges
                        .iter()
                        .map(|edge| edge.source_port.as_str())
                        .collect::<HashSet<_>>();
                    if !ports.contains("approved")
                        || ports
                            .iter()
                            .any(|port| !matches!(*port, "approved" | "denied"))
                        || ports.len() != node_edges.len()
                    {
                        errors.push(format!(
                            "permission {} requires one approved edge and at most one denied edge",
                            node.id()
                        ));
                    }
                    if matches!(permission.on_denied, PermissionDeniedBehavior::Branch)
                        && !ports.contains("denied")
                    {
                        errors.push(format!(
                            "permission {} must define a denied edge when onDenied is branch",
                            node.id()
                        ));
                    }
                }
                WorkflowNode::Output(_) => {
                    require_incoming(node.id(), in_count, &mut errors);
                    if !node_edges.is_empty() {
                        errors.push(format!(
                            "output node {} cannot have outgoing edges",
                            node.id()
                        ));
                    }
                }
            }
        }

        if node_ids.len() == self.nodes.len() {
            validate_dag_and_reachability(
                &node_ids,
                &input_ids,
                &output_ids,
                &self.edges,
                &mut errors,
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_workflow_header(workflow: &WorkflowIr, errors: &mut Vec<String>) {
    if workflow.schema_version != WORKFLOW_IR_SCHEMA_VERSION {
        errors.push(format!(
            "schemaVersion must be {WORKFLOW_IR_SCHEMA_VERSION}"
        ));
    }
    if workflow.compiler.model != WORKFLOW_COMPILER_MODEL
        && workflow.compiler.model != LEGACY_WORKFLOW_COMPILER_MODEL
    {
        errors.push(format!(
            "compiler.model must be {WORKFLOW_COMPILER_MODEL} or the historical {LEGACY_WORKFLOW_COMPILER_MODEL} identity"
        ));
    }
    if workflow.workflow_id.trim().is_empty() || workflow.name.trim().is_empty() {
        errors.push("workflowId and name must not be empty".to_string());
    }
    if workflow.workflow_version == 0 {
        errors.push("workflowVersion must be greater than zero".to_string());
    }
}

fn default_system_action_timeout_ms() -> u64 {
    SHORT_TIMEOUT_MS
}

fn default_system_action_max_output_bytes() -> usize {
    50 * 1024
}

fn default_loop_item_variable() -> String {
    "item".to_string()
}

fn require_incoming(node_id: &str, count: usize, errors: &mut Vec<String>) {
    if count == 0 {
        errors.push(format!("node {node_id} must have an incoming edge"));
    }
}

fn require_system_timeout(node_id: &str, timeout_ms: Option<u64>, errors: &mut Vec<String>) {
    if timeout_ms == Some(0) {
        errors.push(format!(
            "node {node_id} requires systemTimeoutMs greater than zero"
        ));
    }
}

fn require_single_port(
    node_id: &str,
    edges: &[&WorkflowEdge],
    port: &str,
    errors: &mut Vec<String>,
) {
    if edges.is_empty() || edges.iter().any(|edge| edge.source_port != port) {
        errors.push(format!(
            "node {node_id} requires at least one outgoing edge on port {port}"
        ));
    }
}

fn require_exact_ports(
    node_id: &str,
    edges: &[&WorkflowEdge],
    expected: &[&str],
    message: &str,
    errors: &mut Vec<String>,
) {
    let ports = edges
        .iter()
        .map(|edge| edge.source_port.as_str())
        .collect::<HashSet<_>>();
    let expected_ports = expected.iter().copied().collect::<HashSet<_>>();
    if ports != expected_ports || edges.len() != expected.len() {
        errors.push(format!("{node_id}: {message}"));
    }
}

fn validate_dag_and_reachability(
    node_ids: &HashSet<&str>,
    input_ids: &[&str],
    output_ids: &[&str],
    edges: &[WorkflowEdge],
    errors: &mut Vec<String>,
) {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut indegree = node_ids
        .iter()
        .map(|id| (*id, 0usize))
        .collect::<HashMap<_, _>>();
    for edge in edges {
        if node_ids.contains(edge.source_node_id.as_str())
            && node_ids.contains(edge.target_node_id.as_str())
        {
            adjacency
                .entry(&edge.source_node_id)
                .or_default()
                .push(&edge.target_node_id);
            reverse
                .entry(&edge.target_node_id)
                .or_default()
                .push(&edge.source_node_id);
            *indegree.entry(&edge.target_node_id).or_default() += 1;
        }
    }

    let mut queue = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<VecDeque<_>>();
    let mut visited_count = 0;
    while let Some(node_id) = queue.pop_front() {
        visited_count += 1;
        for target in adjacency.get(node_id).into_iter().flatten() {
            let Some(degree) = indegree.get_mut(target) else {
                errors.push(format!(
                    "workflow edge points to unknown target node '{}'",
                    target
                ));
                continue;
            };
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(target);
            }
        }
    }
    if visited_count != node_ids.len() {
        errors.push("workflow graph must be acyclic".to_string());
    }

    let reachable_from_inputs = traverse(input_ids, &adjacency);
    let can_reach_outputs = traverse(output_ids, &reverse);
    for node_id in node_ids {
        if !reachable_from_inputs.contains(node_id) {
            errors.push(format!("node {node_id} is not reachable from an input"));
        }
        if !can_reach_outputs.contains(node_id) {
            errors.push(format!("node {node_id} cannot reach an output"));
        }
    }
}

fn traverse<'a>(starts: &[&'a str], graph: &HashMap<&'a str, Vec<&'a str>>) -> HashSet<&'a str> {
    let mut visited = HashSet::new();
    let mut queue = starts.iter().copied().collect::<VecDeque<_>>();
    while let Some(node_id) = queue.pop_front() {
        if !visited.insert(node_id) {
            continue;
        }
        queue.extend(graph.get(node_id).into_iter().flatten().copied());
    }
    visited
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_ir() -> WorkflowIr {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": "1.0.0",
            "workflowId": "wf-test",
            "workflowVersion": 1,
            "name": "Permission flow",
            "description": "",
            "compiler": { "model": "gemma-4-e4b-qat" },
            "nodes": [
                {
                    "kind": "input", "id": "input", "label": "Input",
                    "outputKey": "request", "inputSchema": {"type": "object"}
                },
                {
                    "kind": "permission", "id": "gate", "label": "Approve",
                    "permission": "file_write", "reason": "Writes a report",
                    "onDenied": "branch"
                },
                {
                    "kind": "output", "id": "approved", "label": "Approved",
                    "inputMapping": "$.gate", "outputSchema": {"type": "object"}
                },
                {
                    "kind": "output", "id": "denied", "label": "Denied",
                    "inputMapping": "$.gate", "outputSchema": {"type": "object"}
                }
            ],
            "edges": [
                {
                    "id": "e1", "sourceNodeId": "input", "sourcePort": "out",
                    "targetNodeId": "gate"
                },
                {
                    "id": "e2", "sourceNodeId": "gate", "sourcePort": "approved",
                    "targetNodeId": "approved"
                },
                {
                    "id": "e3", "sourceNodeId": "gate", "sourcePort": "denied",
                    "targetNodeId": "denied"
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn accepts_a_connected_acyclic_permission_flow() {
        assert_eq!(valid_ir().validate(), Ok(()));
    }

    #[test]
    fn accepts_legacy_e2b_identity_for_historical_workflow_reads_only() {
        let mut ir = valid_ir();
        ir.compiler.model = LEGACY_WORKFLOW_COMPILER_MODEL.to_string();
        assert_eq!(ir.validate(), Ok(()));

        ir.compiler.model = "unknown-compiler".to_string();
        let errors = ir.validate().unwrap_err().join("\n");
        assert!(errors.contains(WORKFLOW_COMPILER_MODEL));
        assert!(errors.contains(LEGACY_WORKFLOW_COMPILER_MODEL));
    }

    #[test]
    fn output_completion_kind_is_backward_compatible_and_explicit() {
        let ir = valid_ir();
        let default_output = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::Output(output) => Some(output),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            default_output.completion_kind,
            WorkflowCompletionKind::Result
        );

        let empty_output: OutputNode = serde_json::from_value(serde_json::json!({
            "id": "empty-output",
            "label": "Nothing found",
            "inputMapping": "{{nodes.read.output.data.items}}",
            "outputSchema": {"type": "array"},
            "completionKind": "empty_collection"
        }))
        .unwrap();
        assert_eq!(
            empty_output.completion_kind,
            WorkflowCompletionKind::EmptyCollection
        );
    }

    #[test]
    fn rejects_cycles_and_unreachable_outputs() {
        let mut ir = valid_ir();
        ir.edges.push(WorkflowEdge {
            id: "cycle".to_string(),
            source_node_id: "approved".to_string(),
            source_port: "out".to_string(),
            target_node_id: "gate".to_string(),
            target_port: None,
        });
        let errors = ir.validate().unwrap_err().join("\n");
        assert!(errors.contains("acyclic"));
        assert!(errors.contains("output node approved cannot have outgoing edges"));
    }

    #[test]
    fn rejects_unknown_fields_during_deserialization() {
        let result = serde_json::from_value::<WorkflowIr>(serde_json::json!({
            "schemaVersion": "1.0.0",
            "workflowId": "wf-test",
            "workflowVersion": 1,
            "name": "Bad",
            "compiler": { "model": "gemma-4-e2b-qat", "extra": true },
            "nodes": [],
            "edges": []
        }));
        assert!(result.is_err());
    }
}
