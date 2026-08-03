import { defaultArgumentsForSchema } from "./McpSchemaFields";
import type {
  CapabilityCatalog,
  WorkflowCapabilityAction,
} from "./workflowCapabilityCatalog";
import {
  MEDIUM_TIMEOUT_MS,
  SHORT_TIMEOUT_MS,
  workflowIrSchema,
  type WorkflowIr,
  type WorkflowIrEdge,
  type WorkflowIrNode,
} from "./workflowIr";

export function updateWorkflowIrNode(
  workflowIr: WorkflowIr,
  nodeId: string,
  patch: Partial<WorkflowIrNode>,
) {
  return parseWorkflowIr({
    ...workflowIr,
    nodes: workflowIr.nodes.map((node) =>
      node.id === nodeId ? ({ ...node, ...patch } as WorkflowIrNode) : node,
    ),
  });
}

export function updateWorkflowIrEdgeTarget(
  workflowIr: WorkflowIr,
  edgeId: string,
  targetNodeId: string,
) {
  return parseWorkflowIr({
    ...workflowIr,
    edges: workflowIr.edges.map((edge) =>
      edge.id === edgeId ? { ...edge, targetNodeId } : edge,
    ),
  });
}

export function insertCapabilityStep(
  workflowIr: WorkflowIr,
  afterNodeId: string,
  action: WorkflowCapabilityAction,
) {
  const source = workflowIr.nodes.find((node) => node.id === afterNodeId);
  if (!source) throw new Error(`Unknown source node: ${afterNodeId}`);
  const outgoing = workflowIr.edges.filter((edge) => edge.sourceNodeId === afterNodeId);
  if (outgoing.length !== 1) throw new Error("Add step needs one clear outgoing path.");
  const node = workflowIrNodeFromCapability(workflowIr, action, source);
  if (!node) throw new Error("This action cannot be inserted as a workflow step yet.");
  return insertNodeBetween(workflowIr, outgoing[0], node);
}

export function addConditionalBranch(
  workflowIr: WorkflowIr,
  afterNodeId: string,
  condition: string,
  labels: {
    branchLabel?: string;
    defaultCondition?: string;
    fallbackLabel?: string;
    fallbackObjective?: string;
  } = {},
) {
  const source = workflowIr.nodes.find((node) => node.id === afterNodeId);
  if (!source) throw new Error(`Unknown source node: ${afterNodeId}`);
  const outgoing = workflowIr.edges.filter((edge) => edge.sourceNodeId === afterNodeId);
  if (outgoing.length !== 1) throw new Error("Add branch needs one clear outgoing path.");

  const conditionId = uniqueNodeId(workflowIr, `${slug(source.label)}-branch`);
  const fallbackId = uniqueNodeId(workflowIr, `${slug(source.label)}-otherwise`);
  const conditional: WorkflowIrNode = {
    kind: "conditional",
    id: conditionId,
    label: labels.branchLabel || "Branch",
    condition: condition.trim() || labels.defaultCondition || "The condition matches.",
    inputMapping: outputReference(source),
    systemTimeoutMs: MEDIUM_TIMEOUT_MS,
  };
  const fallback: WorkflowIrNode = {
    kind: "agent",
    id: fallbackId,
    label: labels.fallbackLabel || "Otherwise",
    objective:
      labels.fallbackObjective ||
      "Handle the path when the branch condition does not match.",
    inputMappings: { context: outputReference(source) },
    outputKey: `nodes.${fallbackId}.output`,
    systemTimeoutMs: MEDIUM_TIMEOUT_MS,
  };
  const replaced = outgoing[0];
  const insertIndex = Math.max(
    0,
    workflowIr.nodes.findIndex((node) => node.id === replaced.targetNodeId),
  );
  const nodes = [
    ...workflowIr.nodes.slice(0, insertIndex),
    conditional,
    fallback,
    ...workflowIr.nodes.slice(insertIndex),
  ];
  const edges = [
    ...workflowIr.edges.filter((edge) => edge.id !== replaced.id),
    edge(workflowIr, source.id, replaced.sourcePort, conditionId),
    edge(workflowIr, conditionId, "true", replaced.targetNodeId),
    edge(workflowIr, conditionId, "false", fallbackId),
    edge(workflowIr, fallbackId, "out", replaced.targetNodeId),
  ];
  return parseWorkflowIr({ ...workflowIr, nodes, edges });
}

export function removeWorkflowIrNode(workflowIr: WorkflowIr, nodeId: string) {
  const node = workflowIr.nodes.find((entry) => entry.id === nodeId);
  if (!node || node.kind === "input" || node.kind === "output") {
    throw new Error("This step cannot be removed.");
  }
  const incoming = workflowIr.edges.filter((entry) => entry.targetNodeId === nodeId);
  const outgoing = workflowIr.edges.filter((entry) => entry.sourceNodeId === nodeId);
  if (outgoing.length !== 1) {
    throw new Error("Only steps with one outgoing path can be removed inline.");
  }
  const targetNodeId = outgoing[0].targetNodeId;
  const edges = [
    ...workflowIr.edges.filter(
      (entry) => entry.sourceNodeId !== nodeId && entry.targetNodeId !== nodeId,
    ),
    ...incoming.map((entry) =>
      edge(workflowIr, entry.sourceNodeId, entry.sourcePort, targetNodeId),
    ),
  ];
  return parseWorkflowIr({
    ...workflowIr,
    nodes: workflowIr.nodes.filter((entry) => entry.id !== nodeId),
    edges,
  });
}

export function capabilityActionsForAddStep(catalog?: CapabilityCatalog | null) {
  return (catalog?.actions ?? [])
    .filter((action) => action.available)
    .filter(
      (action) =>
        action.kind === "mcp_tool" ||
        action.kind === "system_action" ||
        action.kind === "agent",
    )
    .sort((left, right) => left.title.localeCompare(right.title));
}

export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function insertNodeBetween(
  workflowIr: WorkflowIr,
  replacedEdge: WorkflowIrEdge,
  node: WorkflowIrNode,
) {
  const insertIndex = Math.max(
    0,
    workflowIr.nodes.findIndex((entry) => entry.id === replacedEdge.targetNodeId),
  );
  const nodes = [
    ...workflowIr.nodes.slice(0, insertIndex),
    node,
    ...workflowIr.nodes.slice(insertIndex),
  ];
  const edges = [
    ...workflowIr.edges.filter((entry) => entry.id !== replacedEdge.id),
    edge(
      workflowIr,
      replacedEdge.sourceNodeId,
      replacedEdge.sourcePort,
      node.id,
    ),
    edge(workflowIr, node.id, defaultSourcePort(node), replacedEdge.targetNodeId),
  ];
  return parseWorkflowIr({ ...workflowIr, nodes, edges });
}

function workflowIrNodeFromCapability(
  workflowIr: WorkflowIr,
  action: WorkflowCapabilityAction,
  source: WorkflowIrNode,
): WorkflowIrNode | null {
  const id = uniqueNodeId(workflowIr, slug(action.title || action.id));
  if (action.kind === "mcp_tool" && action.serverName && action.toolName) {
    return {
      kind: "mcp_tool",
      id,
      label: action.title,
      serverName: action.serverName,
      toolName: action.toolName,
      arguments: defaultArgumentsForSchema(action.inputSchema),
      inputSchema: action.inputSchema,
      outputSchema: action.outputSchema,
      systemTimeoutMs: SHORT_TIMEOUT_MS,
    };
  }
  if (action.kind === "system_action") {
    const template = asRecord(action.nodeTemplate);
    return {
      kind: "system_action",
      id,
      label: action.title,
      actionType:
        template.actionType === "python" || template.actionType === "binary"
          ? template.actionType
          : "shell",
      command:
        typeof template.command === "string" && template.command.trim()
          ? template.command.trim()
          : "echo",
      args: Array.isArray(template.args) ? template.args.map(String) : [],
      workingDirectory:
        typeof template.workingDirectory === "string"
          ? template.workingDirectory
          : undefined,
      systemTimeoutMs: SHORT_TIMEOUT_MS,
      timeoutMs: SHORT_TIMEOUT_MS,
      maxOutputBytes: 51200,
    };
  }
  if (action.kind === "agent") {
    return {
      kind: "agent",
      id,
      label: action.title,
      objective: action.outcome || action.detail || action.title,
      inputMappings: { context: outputReference(source) },
      outputKey: `nodes.${id}.output`,
      systemTimeoutMs: MEDIUM_TIMEOUT_MS,
    };
  }
  return null;
}

function parseWorkflowIr(value: WorkflowIr) {
  return workflowIrSchema.parse(value);
}

function outputReference(node: WorkflowIrNode) {
  if (node.kind === "input") return "{{workflow.input}}";
  if ("outputKey" in node) return `{{${node.outputKey}}}`;
  return `{{nodes.${node.id}.output}}`;
}

function defaultSourcePort(node: WorkflowIrNode) {
  return node.kind === "permission" ? "approved" : "out";
}

function edge(
  workflowIr: WorkflowIr,
  sourceNodeId: string,
  sourcePort: string,
  targetNodeId: string,
) {
  return {
    id: uniqueEdgeId(workflowIr, sourceNodeId, sourcePort, targetNodeId),
    sourceNodeId,
    sourcePort,
    targetNodeId,
  };
}

function uniqueNodeId(workflowIr: WorkflowIr, base: string) {
  const existing = new Set(workflowIr.nodes.map((node) => node.id));
  const root = base || "step";
  if (!existing.has(root)) return root;
  for (let index = 2; index < 1000; index += 1) {
    const candidate = `${root}-${index}`;
    if (!existing.has(candidate)) return candidate;
  }
  return `${root}-${Date.now()}`;
}

function uniqueEdgeId(
  workflowIr: WorkflowIr,
  sourceNodeId: string,
  sourcePort: string,
  targetNodeId: string,
) {
  const existing = new Set(workflowIr.edges.map((entry) => entry.id));
  const root = `edge-${slug(sourceNodeId)}-${slug(sourcePort)}-${slug(targetNodeId)}`;
  if (!existing.has(root)) return root;
  for (let index = 2; index < 1000; index += 1) {
    const candidate = `${root}-${index}`;
    if (!existing.has(candidate)) return candidate;
  }
  return `${root}-${Date.now()}`;
}

function slug(value: string) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "")
    .slice(0, 48);
}
