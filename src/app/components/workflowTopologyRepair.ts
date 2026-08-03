import {
  SHORT_TIMEOUT_MS,
  workflowIrSchema,
  type WorkflowIr,
  type WorkflowIrEdge,
  type WorkflowIrNode,
} from "./workflowIr";

const TASKFLOW_NATIVE_SERVER_NAME = "taskflow_native";
const TASKFLOW_DEFAULT_REPORT_PATH = "workspace/report.md";

export function insertMissingReportWriter(workflowIr: WorkflowIr): WorkflowIr | null {
  const previewNode = workflowIr.nodes.find(
    (node) =>
      node.kind === "mcp_tool" &&
      node.toolName === "preview_report" &&
      !hasUpstreamReportWriter(workflowIr, node.id),
  );
  if (!previewNode) {
    return null;
  }

  const incomingEdge = workflowIr.edges.find(
    (edge) => edge.targetNodeId === previewNode.id,
  );
  if (!incomingEdge) {
    return null;
  }

  const usedNodeIds = new Set(workflowIr.nodes.map((node) => node.id));
  const usedEdgeIds = new Set(workflowIr.edges.map((edge) => edge.id));
  const writerId = uniqueWorkflowNodeId("write-report", usedNodeIds);
  const contentSourceId =
    reportContentSourceNodeId(workflowIr, incomingEdge.sourceNodeId) ??
    incomingEdge.sourceNodeId;
  const writerNode: WorkflowIrNode = {
    kind: "mcp_tool",
    id: writerId,
    label: "Write a project report",
    serverName: TASKFLOW_NATIVE_SERVER_NAME,
    toolName: "write_markdown_report",
    arguments: {
      reportPath: TASKFLOW_DEFAULT_REPORT_PATH,
      content: workflowReferenceForNode(workflowIr, contentSourceId),
    },
    inputSchema: {
      type: "object",
      properties: {
        reportPath: { type: "string" },
        content: { type: "string" },
      },
      required: ["reportPath", "content"],
      additionalProperties: false,
    },
    systemTimeoutMs: SHORT_TIMEOUT_MS,
  };

  const nodes = workflowIr.nodes.flatMap((node): WorkflowIrNode[] => {
    if (node.id !== previewNode.id || node.kind !== "mcp_tool") {
      return [node];
    }
    return [
      writerNode,
      {
        ...node,
        arguments: {
          ...recordFromUnknown(node.arguments),
          reportPath: TASKFLOW_DEFAULT_REPORT_PATH,
        },
      },
    ];
  });
  const reroutedEdges = workflowIr.edges.map((edge): WorkflowIrEdge => {
    if (edge.id !== incomingEdge.id) {
      return edge;
    }
    return {
      ...edge,
      targetNodeId: writerId,
      targetPort: undefined,
    };
  });
  const writerEdge: WorkflowIrEdge = {
    id: uniqueWorkflowEdgeId(`edge-${writerId}-${previewNode.id}`, usedEdgeIds),
    sourceNodeId: writerId,
    sourcePort: "out",
    targetNodeId: previewNode.id,
  };

  const parsed = workflowIrSchema.safeParse({
    ...workflowIr,
    nodes,
    edges: [...reroutedEdges, writerEdge],
  });
  return parsed.success ? parsed.data : null;
}

function hasUpstreamReportWriter(workflowIr: WorkflowIr, targetNodeId: string) {
  const nodeById = new Map(workflowIr.nodes.map((node) => [node.id, node]));
  const reverseEdges = new Map<string, string[]>();
  workflowIr.edges.forEach((edge) => {
    reverseEdges.set(edge.targetNodeId, [
      ...(reverseEdges.get(edge.targetNodeId) ?? []),
      edge.sourceNodeId,
    ]);
  });

  const seen = new Set<string>();
  const stack = [...(reverseEdges.get(targetNodeId) ?? [])];
  while (stack.length > 0) {
    const nodeId = stack.pop();
    if (!nodeId || seen.has(nodeId)) {
      continue;
    }
    seen.add(nodeId);
    const node = nodeById.get(nodeId);
    if (
      node?.kind === "mcp_tool" &&
      ["write_markdown_report", "write_file"].includes(node.toolName)
    ) {
      return true;
    }
    stack.push(...(reverseEdges.get(nodeId) ?? []));
  }
  return false;
}

function reportContentSourceNodeId(
  workflowIr: WorkflowIr,
  sourceNodeId: string,
) {
  const node = workflowIr.nodes.find(
    (candidate) => candidate.id === sourceNodeId,
  );
  if (node?.kind !== "permission") {
    return sourceNodeId;
  }
  return workflowIr.edges.find((edge) => edge.targetNodeId === sourceNodeId)
    ?.sourceNodeId;
}

function workflowReferenceForNode(workflowIr: WorkflowIr, nodeId: string) {
  const node = workflowIr.nodes.find((candidate) => candidate.id === nodeId);
  return node?.kind === "input"
    ? "{{workflow.input}}"
    : `{{nodes.${nodeId}.output}}`;
}

function uniqueWorkflowNodeId(baseId: string, usedIds: Set<string>) {
  if (!usedIds.has(baseId)) {
    usedIds.add(baseId);
    return baseId;
  }
  for (let index = 2; ; index += 1) {
    const candidate = `${baseId}-${index}`;
    if (!usedIds.has(candidate)) {
      usedIds.add(candidate);
      return candidate;
    }
  }
}

function uniqueWorkflowEdgeId(baseId: string, usedIds: Set<string>) {
  if (!usedIds.has(baseId)) {
    usedIds.add(baseId);
    return baseId;
  }
  for (let index = 2; ; index += 1) {
    const candidate = `${baseId}-${index}`;
    if (!usedIds.has(candidate)) {
      usedIds.add(candidate);
      return candidate;
    }
  }
}

function recordFromUnknown(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? { ...(value as Record<string, unknown>) }
    : {};
}
