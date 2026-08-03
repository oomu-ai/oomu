import { z } from "zod";

export const WORKFLOW_IR_SCHEMA_VERSION = "1.0.0" as const;
export const WORKFLOW_COMPILER_MODEL = "gemma-4-e4b-qat" as const;
export const LEGACY_WORKFLOW_COMPILER_MODEL = "gemma-4-e2b-qat" as const;
export const SHORT_TIMEOUT_MS = 10_000;
export const MEDIUM_TIMEOUT_MS = 60_000;
const LONG_TIMEOUT_MS = 300_000;

const nonEmptyString = z.string().trim().min(1);
const jsonSchemaObject = z.record(z.string(), z.unknown());
const systemTimeoutMsSchema = z.number().int().positive().max(LONG_TIMEOUT_MS);

const inputNodeSchema = z.strictObject({
  kind: z.literal("input"),
  id: nonEmptyString,
  label: nonEmptyString,
  outputKey: nonEmptyString,
  inputSchema: jsonSchemaObject,
});

const agentNodeSchema = z.strictObject({
  kind: z.literal("agent"),
  id: nonEmptyString,
  label: nonEmptyString,
  objective: nonEmptyString,
  inputMappings: z.record(z.string(), nonEmptyString).default({}),
  outputKey: nonEmptyString,
  systemTimeoutMs: systemTimeoutMsSchema.default(MEDIUM_TIMEOUT_MS),
});

const routerNodeSchema = z.strictObject({
  kind: z.literal("router"),
  id: nonEmptyString,
  label: nonEmptyString,
  expression: nonEmptyString,
  routes: z
    .array(
      z.strictObject({
        port: nonEmptyString,
        condition: nonEmptyString,
      }),
    )
    .min(2),
  systemTimeoutMs: systemTimeoutMsSchema.default(MEDIUM_TIMEOUT_MS),
});

const conditionalNodeSchema = z.strictObject({
  kind: z.literal("conditional"),
  id: nonEmptyString,
  label: nonEmptyString,
  condition: nonEmptyString,
  inputMapping: nonEmptyString.optional(),
  systemTimeoutMs: systemTimeoutMsSchema.default(MEDIUM_TIMEOUT_MS),
});

const loopNodeSchema = z.strictObject({
  kind: z.literal("loop"),
  id: nonEmptyString,
  label: nonEmptyString,
  itemsMapping: nonEmptyString,
  itemVariable: nonEmptyString.default("item"),
  systemTimeoutMs: systemTimeoutMsSchema.default(MEDIUM_TIMEOUT_MS),
});

const permissionNodeSchema = z.strictObject({
  kind: z.literal("permission"),
  id: nonEmptyString,
  label: nonEmptyString,
  permission: z.enum([
    "file_read",
    "file_write",
    "network",
    "process",
    "mcp_tool",
    "custom",
  ]),
  reason: nonEmptyString,
  onDenied: z.enum(["fail", "branch"]),
});

const mcpToolNodeSchema = z.strictObject({
  kind: z.literal("mcp_tool"),
  id: nonEmptyString,
  label: nonEmptyString,
  serverName: nonEmptyString,
  toolName: nonEmptyString,
  arguments: z.unknown().default({}),
  inputSchema: z.unknown().optional(),
  outputSchema: z.unknown().optional(),
  systemTimeoutMs: systemTimeoutMsSchema.default(MEDIUM_TIMEOUT_MS),
});

const systemActionNodeSchema = z.strictObject({
  kind: z.literal("system_action"),
  id: nonEmptyString,
  label: nonEmptyString,
  actionType: z.enum(["shell", "python", "binary"]),
  command: nonEmptyString,
  args: z.array(z.string()).default([]),
  workingDirectory: z.string().optional(),
  systemTimeoutMs: systemTimeoutMsSchema.default(SHORT_TIMEOUT_MS),
  timeoutMs: systemTimeoutMsSchema.default(SHORT_TIMEOUT_MS),
  maxOutputBytes: z.number().int().positive().max(51200).default(51200),
});

const outputNodeSchema = z.strictObject({
  kind: z.literal("output"),
  id: nonEmptyString,
  label: nonEmptyString,
  inputMapping: nonEmptyString,
  outputSchema: jsonSchemaObject,
  completionKind: z.enum(["result", "empty_collection"]).optional(),
});

const workflowIrNodeSchema = z.discriminatedUnion("kind", [
  inputNodeSchema,
  agentNodeSchema,
  routerNodeSchema,
  conditionalNodeSchema,
  loopNodeSchema,
  permissionNodeSchema,
  mcpToolNodeSchema,
  systemActionNodeSchema,
  outputNodeSchema,
]);

const workflowIrEdgeSchema = z.strictObject({
  id: nonEmptyString,
  sourceNodeId: nonEmptyString,
  sourcePort: nonEmptyString,
  targetNodeId: nonEmptyString,
  targetPort: nonEmptyString.optional(),
});

const workflowIrStructuralSchema = z.strictObject({
  schemaVersion: z.literal(WORKFLOW_IR_SCHEMA_VERSION),
  workflowId: nonEmptyString,
  workflowVersion: z.number().int().positive(),
  name: nonEmptyString,
  description: z.string().default(""),
  compiler: z.strictObject({
    model: z.enum([WORKFLOW_COMPILER_MODEL, LEGACY_WORKFLOW_COMPILER_MODEL]),
  }),
  metadata: z.record(z.string(), z.unknown()).optional(),
  nodes: z.array(workflowIrNodeSchema).min(2),
  edges: z.array(workflowIrEdgeSchema).min(1),
});

export const workflowIrSchema = workflowIrStructuralSchema.superRefine(
  (workflow, context) => {
    const nodeById = new Map(workflow.nodes.map((node) => [node.id, node]));
    const nodeIds = new Set<string>();
    const edgeIds = new Set<string>();
    const edgeSignatures = new Set<string>();
    const incoming = new Map<string, number>();
    const outgoing = new Map<string, typeof workflow.edges>();

    workflow.nodes.forEach((node, index) => {
      if (nodeIds.has(node.id)) {
        issue(context, ["nodes", index, "id"], `Duplicate node id: ${node.id}`);
      }
      nodeIds.add(node.id);
    });

    workflow.edges.forEach((edge, index) => {
      if (edgeIds.has(edge.id)) {
        issue(context, ["edges", index, "id"], `Duplicate edge id: ${edge.id}`);
      }
      edgeIds.add(edge.id);

      const signature = [
        edge.sourceNodeId,
        edge.sourcePort,
        edge.targetNodeId,
        edge.targetPort ?? "",
      ].join("\u0000");
      if (edgeSignatures.has(signature)) {
        issue(context, ["edges", index], "Duplicate edge connection");
      }
      edgeSignatures.add(signature);

      if (!nodeById.has(edge.sourceNodeId)) {
        issue(context, ["edges", index, "sourceNodeId"], "Unknown source node");
      }
      if (!nodeById.has(edge.targetNodeId)) {
        issue(context, ["edges", index, "targetNodeId"], "Unknown target node");
      }
      if (edge.sourceNodeId === edge.targetNodeId) {
        issue(context, ["edges", index], "Self edges are not allowed");
      }

      incoming.set(edge.targetNodeId, (incoming.get(edge.targetNodeId) ?? 0) + 1);
      outgoing.set(edge.sourceNodeId, [
        ...(outgoing.get(edge.sourceNodeId) ?? []),
        edge,
      ]);
    });

    const inputs = workflow.nodes.filter((node) => node.kind === "input");
    const outputs = workflow.nodes.filter((node) => node.kind === "output");
    if (inputs.length === 0) {
      issue(context, ["nodes"], "Workflow requires at least one input node");
    }
    if (outputs.length === 0) {
      issue(context, ["nodes"], "Workflow requires at least one output node");
    }

    workflow.nodes.forEach((node, index) => {
      const inCount = incoming.get(node.id) ?? 0;
      const nodeEdges = outgoing.get(node.id) ?? [];
      if (node.kind === "input") {
        if (inCount > 0) {
          issue(context, ["nodes", index], "Input nodes cannot have incoming edges");
        }
        validateStandardOutput(context, node.id, nodeEdges, index);
      } else if (node.kind === "output") {
        requireIncoming(context, node.id, inCount, index);
        if (nodeEdges.length > 0) {
          issue(context, ["nodes", index], "Output nodes cannot have outgoing edges");
        }
      } else if (node.kind === "router") {
        requireIncoming(context, node.id, inCount, index);
        const routePorts = new Set(node.routes.map((route) => route.port));
        const edgePorts = new Set(nodeEdges.map((edge) => edge.sourcePort));
        if (routePorts.size !== node.routes.length) {
          issue(context, ["nodes", index, "routes"], "Router route ports must be unique");
        }
        if (
          routePorts.size !== edgePorts.size ||
          nodeEdges.length !== routePorts.size ||
          [...routePorts].some((port) => !edgePorts.has(port))
        ) {
          issue(
            context,
            ["nodes", index],
            "Router requires exactly one outgoing edge for every route port",
          );
        }
      } else if (node.kind === "conditional") {
        requireIncoming(context, node.id, inCount, index);
        validateExactPorts(
          context,
          node.id,
          nodeEdges,
          ["true", "false"],
          index,
          "Conditional nodes require one true edge and one false edge",
        );
      } else if (node.kind === "loop") {
        requireIncoming(context, node.id, inCount, index);
        validateExactPorts(
          context,
          node.id,
          nodeEdges,
          ["item", "done"],
          index,
          "Loop nodes require one item edge and one done edge",
        );
      } else if (node.kind === "permission") {
        requireIncoming(context, node.id, inCount, index);
        const ports = new Set(nodeEdges.map((edge) => edge.sourcePort));
        if (
          !ports.has("approved") ||
          [...ports].some((port) => port !== "approved" && port !== "denied") ||
          ports.size !== nodeEdges.length
        ) {
          issue(
            context,
            ["nodes", index],
            "Permission nodes require one approved edge and at most one denied edge",
          );
        }
        if (node.onDenied === "branch" && !ports.has("denied")) {
          issue(
            context,
            ["nodes", index],
            "Permission nodes with onDenied=branch require a denied edge",
          );
        }
      } else {
        requireIncoming(context, node.id, inCount, index);
        validateStandardOutput(context, node.id, nodeEdges, index);
      }
    });

    if (nodeIds.size !== workflow.nodes.length) {
      return;
    }

    const adjacency = new Map<string, string[]>();
    const reverse = new Map<string, string[]>();
    const indegree = new Map(workflow.nodes.map((node) => [node.id, 0]));
    workflow.edges.forEach((edge) => {
      if (!nodeById.has(edge.sourceNodeId) || !nodeById.has(edge.targetNodeId)) {
        return;
      }
      adjacency.set(edge.sourceNodeId, [
        ...(adjacency.get(edge.sourceNodeId) ?? []),
        edge.targetNodeId,
      ]);
      reverse.set(edge.targetNodeId, [
        ...(reverse.get(edge.targetNodeId) ?? []),
        edge.sourceNodeId,
      ]);
      indegree.set(edge.targetNodeId, (indegree.get(edge.targetNodeId) ?? 0) + 1);
    });

    const queue = [...indegree]
      .filter(([, degree]) => degree === 0)
      .map(([nodeId]) => nodeId);
    let visitedCount = 0;
    for (let cursor = 0; cursor < queue.length; cursor += 1) {
      const nodeId = queue[cursor];
      visitedCount += 1;
      for (const target of adjacency.get(nodeId) ?? []) {
        const degree = (indegree.get(target) ?? 0) - 1;
        indegree.set(target, degree);
        if (degree === 0) {
          queue.push(target);
        }
      }
    }
    if (visitedCount !== workflow.nodes.length) {
      issue(context, ["edges"], "Workflow graph must be acyclic");
    }

    const reachableFromInputs = traverse(
      inputs.map((node) => node.id),
      adjacency,
    );
    const canReachOutputs = traverse(
      outputs.map((node) => node.id),
      reverse,
    );
    workflow.nodes.forEach((node, index) => {
      if (!reachableFromInputs.has(node.id)) {
        issue(context, ["nodes", index], "Node is not reachable from an input");
      }
      if (!canReachOutputs.has(node.id)) {
        issue(context, ["nodes", index], "Node cannot reach an output");
      }
    });
  },
);

function requireIncoming(
  context: z.RefinementCtx,
  nodeId: string,
  count: number,
  index: number,
) {
  if (count === 0) {
    issue(context, ["nodes", index], `Node ${nodeId} requires an incoming edge`);
  }
}

function validateExactPorts(
  context: z.RefinementCtx,
  nodeId: string,
  edges: z.infer<typeof workflowIrEdgeSchema>[],
  expectedPorts: string[],
  index: number,
  message: string,
) {
  const ports = new Set(edges.map((edge) => edge.sourcePort));
  if (
    ports.size !== expectedPorts.length ||
    edges.length !== expectedPorts.length ||
    expectedPorts.some((port) => !ports.has(port))
  ) {
    issue(context, ["nodes", index], `${nodeId}: ${message}`);
  }
}

function validateStandardOutput(
  context: z.RefinementCtx,
  nodeId: string,
  edges: z.infer<typeof workflowIrEdgeSchema>[],
  index: number,
) {
  if (edges.length === 0 || edges.some((edge) => edge.sourcePort !== "out")) {
    issue(
      context,
      ["nodes", index],
      `Node ${nodeId} requires at least one outgoing edge on port out`,
    );
  }
}

function traverse(starts: string[], graph: Map<string, string[]>) {
  const visited = new Set<string>();
  const queue = [...starts];
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const nodeId = queue[cursor];
    if (visited.has(nodeId)) {
      continue;
    }
    visited.add(nodeId);
    queue.push(...(graph.get(nodeId) ?? []));
  }
  return visited;
}

function issue(
  context: z.RefinementCtx,
  path: PropertyKey[],
  message: string,
) {
  context.addIssue({ code: "custom", path, message });
}

export type WorkflowIr = z.infer<typeof workflowIrSchema>;
export type WorkflowIrNode = z.infer<typeof workflowIrNodeSchema>;
export type WorkflowIrEdge = z.infer<typeof workflowIrEdgeSchema>;
