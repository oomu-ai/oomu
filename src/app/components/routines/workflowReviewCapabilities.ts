export type WorkflowReviewCapabilities = {
  status: "ready" | "unavailable";
  calendarCreate: boolean;
  calendarRead: boolean;
  emailDraft: boolean;
  emailRead: boolean;
  emailSend: boolean;
  officialWeb: boolean;
  projectFileRead: boolean;
  projectFileWrite: boolean;
};

export const unavailableWorkflowReview: WorkflowReviewCapabilities = {
  status: "unavailable",
  calendarCreate: false,
  calendarRead: false,
  emailDraft: false,
  emailRead: false,
  emailSend: false,
  officialWeb: false,
  projectFileRead: false,
  projectFileWrite: false,
};

export function workflowReviewCapabilities(
  steps: string | undefined,
): WorkflowReviewCapabilities {
  if (!steps?.trim()) return unavailableWorkflowReview;
  try {
    const parsed: unknown = JSON.parse(steps);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return unavailableWorkflowReview;
    }
    const record = parsed as Record<string, unknown>;
    const workflowIr = asWorkflowRecord(record.workflowIr ?? record);
    if (
      !Array.isArray(workflowIr.nodes) ||
      workflowIr.nodes.length === 0 ||
      !Array.isArray(workflowIr.edges)
    ) {
      return unavailableWorkflowReview;
    }
    const capabilities: WorkflowReviewCapabilities = {
      ...unavailableWorkflowReview,
      status: "ready",
    };
    for (const value of workflowIr.nodes) {
      if (!value || typeof value !== "object" || Array.isArray(value)) {
        return unavailableWorkflowReview;
      }
      const node = value as Record<string, unknown>;
      if (node.kind !== "mcp_tool") continue;
      const server = typeof node.serverName === "string" ? node.serverName : "";
      const tool = typeof node.toolName === "string" ? node.toolName : "";
      if (isProjectRead(server, tool)) {
        capabilities.projectFileRead = true;
        continue;
      }
      if (isProjectWrite(server, tool)) {
        if (
          server === "oomu_task_tools" &&
          !["md", "pdf"].includes(
            String(asWorkflowRecord(asWorkflowRecord(node.arguments).file).format),
          )
        ) {
          return unavailableWorkflowReview;
        }
        capabilities.projectFileWrite = true;
        continue;
      }
      if (server === "oomu_task_tools" && tool === "fetch_official_page") {
        capabilities.officialWeb = true;
        continue;
      }
      if (
        server === "oomu_task_tools" &&
        [
          "analyze_supplier_exceptions",
          "analyze_project_milestones",
          "validate_evidence_report",
        ].includes(tool)
      ) {
        continue;
      }
      if (
        server === "oomu_task_tools" &&
        tool === "create_conflict_free_calendar_event"
      ) {
        if (!hasDirectApproval(workflowIr, String(node.id), true)) {
          return unavailableWorkflowReview;
        }
        capabilities.calendarCreate = true;
        continue;
      }
      if (server === "oomu_task_tools" && tool === "send_system_email") {
        if (!hasDirectApproval(workflowIr, String(node.id), true)) {
          return unavailableWorkflowReview;
        }
        capabilities.emailSend = true;
        continue;
      }
      if (server === "macos_applescript" && tool === "read_system_calendar") {
        capabilities.calendarRead = true;
        continue;
      }
      if (server === "macos_applescript" && tool === "read_system_emails") {
        capabilities.emailRead = true;
        continue;
      }
      if (server === "macos_applescript" && tool === "draft_system_email") {
        if (!hasDirectApproval(workflowIr, String(node.id), false)) {
          return unavailableWorkflowReview;
        }
        capabilities.emailDraft = true;
        continue;
      }
      return unavailableWorkflowReview;
    }
    return capabilities;
  } catch {
    return unavailableWorkflowReview;
  }
}

function isProjectRead(server: string, tool: string) {
  return (
    (server === "oomu_task_tools" && tool === "read_project_file") ||
    (server === "local_filesystem" &&
      ["read_file", "list_directory"].includes(tool)) ||
    (server === "taskflow_native" &&
      ["folder_read", "preview_report"].includes(tool))
  );
}

function isProjectWrite(server: string, tool: string) {
  return (
    (server === "oomu_task_tools" && tool === "create_file") ||
    (server === "local_filesystem" && tool === "write_file") ||
    (server === "taskflow_native" && tool === "write_markdown_report")
  );
}

function asWorkflowRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function hasDirectApproval(
  workflowIr: Record<string, unknown>,
  effectId: string,
  requireDenialBranch: boolean,
) {
  if (!effectId || !Array.isArray(workflowIr.nodes) || !Array.isArray(workflowIr.edges)) {
    return false;
  }
  const edges = workflowIr.edges.map(asWorkflowRecord);
  const incoming = edges.filter(
    (edge) => edge.targetNodeId === effectId && edge.sourcePort === "approved",
  );
  if (incoming.length !== 1) return false;
  const permissionId = incoming[0].sourceNodeId;
  const permission = workflowIr.nodes
    .map(asWorkflowRecord)
    .find((node) => node.id === permissionId && node.kind === "permission");
  if (
    permission?.permission !== "mcp_tool" ||
    !["branch", "fail"].includes(String(permission.onDenied)) ||
    (requireDenialBranch && permission.onDenied !== "branch")
  ) {
    return false;
  }
  const approved = edges.filter(
    (edge) => edge.sourceNodeId === permissionId && edge.sourcePort === "approved",
  );
  const denied = edges.filter(
    (edge) => edge.sourceNodeId === permissionId && edge.sourcePort === "denied",
  );
  if (approved.length !== 1) return false;
  if (permission.onDenied === "fail") return !requireDenialBranch;
  if (denied.length !== 1) return false;
  const deniedTarget = String(denied[0].targetNodeId ?? "");
  const outputIds = workflowIr.nodes
    .map(asWorkflowRecord)
    .filter((node) => node.kind === "output")
    .map((node) => String(node.id ?? ""));
  return (
    outputIds.some((outputId) => workflowPathExists(edges, deniedTarget, outputId)) &&
    !workflowPathExists(edges, deniedTarget, effectId)
  );
}

function workflowPathExists(
  edges: Record<string, unknown>[],
  start: string,
  target: string,
) {
  if (!start || !target) return false;
  if (start === target) return true;
  const pending = [start];
  const seen = new Set<string>();
  while (pending.length > 0) {
    const nodeId = pending.pop() ?? "";
    if (!nodeId || seen.has(nodeId)) continue;
    seen.add(nodeId);
    for (const edge of edges) {
      if (edge.sourceNodeId !== nodeId) continue;
      const next = String(edge.targetNodeId ?? "");
      if (next === target) return true;
      if (next) pending.push(next);
    }
  }
  return false;
}
