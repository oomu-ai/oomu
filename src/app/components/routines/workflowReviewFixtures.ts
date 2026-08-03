export const simpleWorkflowSteps = JSON.stringify({
  workflowIr: {
    nodes: [
      { kind: "input", id: "input" },
      { kind: "agent", id: "summarize" },
      { kind: "output", id: "output" },
    ],
    edges: [
      { sourceNodeId: "input", sourcePort: "out", targetNodeId: "summarize" },
      { sourceNodeId: "summarize", sourcePort: "out", targetNodeId: "output" },
    ],
  },
});

export const routineFixture = {
  routineId: "routine-1",
  label: "Morning brief",
  projectId: "project-1",
  workflowId: "workflow-1",
  workflowVersion: 1,
  scheduleExpression: "0 9 * * *",
  scheduleKind: "recurring",
  timezone: "America/New_York",
  isActive: true,
  nextRunAtMs: Date.UTC(2026, 6, 13, 13),
  nextRunsMs: [Date.UTC(2026, 6, 13, 13)],
  missedRunPolicy: "skip",
  consecutiveFailures: 0,
  failureThreshold: 3,
  deliveryTarget: {},
};

const scenarioFiveNodes = [
  { kind: "input", id: "input" },
  {
    kind: "mcp_tool",
    id: "read-suppliers",
    serverName: "oomu_task_tools",
    toolName: "read_project_file",
  },
  {
    kind: "mcp_tool",
    id: "analyze-suppliers",
    serverName: "oomu_task_tools",
    toolName: "analyze_supplier_exceptions",
  },
  {
    kind: "mcp_tool",
    id: "read-milestones",
    serverName: "oomu_task_tools",
    toolName: "read_project_file",
  },
  {
    kind: "mcp_tool",
    id: "analyze-milestones",
    serverName: "oomu_task_tools",
    toolName: "analyze_project_milestones",
  },
  {
    kind: "mcp_tool",
    id: "official-source-a",
    serverName: "oomu_task_tools",
    toolName: "fetch_official_page",
  },
  {
    kind: "mcp_tool",
    id: "official-source-b",
    serverName: "oomu_task_tools",
    toolName: "fetch_official_page",
  },
  { kind: "agent", id: "brief" },
  {
    kind: "mcp_tool",
    id: "validate-brief",
    serverName: "oomu_task_tools",
    toolName: "validate_evidence_report",
  },
  {
    kind: "mcp_tool",
    id: "write-md",
    serverName: "oomu_task_tools",
    toolName: "create_file",
    arguments: { file: { format: "md" } },
  },
  {
    kind: "mcp_tool",
    id: "write-pdf",
    serverName: "oomu_task_tools",
    toolName: "create_file",
    arguments: { file: { format: "pdf" } },
  },
  { kind: "output", id: "output" },
];

const serialEdges = (nodeIds: string[]) =>
  nodeIds.slice(0, -1).map((sourceNodeId, index) => ({
    sourceNodeId,
    sourcePort: "out",
    targetNodeId: nodeIds[index + 1],
  }));

export const scenarioFiveWorkflowSteps = JSON.stringify({
  workflowIr: {
    nodes: scenarioFiveNodes,
    edges: serialEdges(scenarioFiveNodes.map((node) => node.id)),
  },
});

const scenarioSixNodes = [
  { kind: "input", id: "input" },
  {
    kind: "mcp_tool",
    id: "read-suppliers",
    serverName: "oomu_task_tools",
    toolName: "read_project_file",
  },
  {
    kind: "mcp_tool",
    id: "analyze-suppliers",
    serverName: "oomu_task_tools",
    toolName: "analyze_supplier_exceptions",
  },
  {
    kind: "mcp_tool",
    id: "source",
    serverName: "oomu_task_tools",
    toolName: "fetch_official_page",
  },
  { kind: "agent", id: "assess" },
  {
    kind: "mcp_tool",
    id: "validate-report",
    serverName: "oomu_task_tools",
    toolName: "validate_evidence_report",
  },
  {
    kind: "mcp_tool",
    id: "write-report",
    serverName: "oomu_task_tools",
    toolName: "create_file",
    arguments: { file: { format: "md" } },
  },
  { kind: "conditional", id: "has-exception" },
  { kind: "output", id: "no-exception" },
  {
    kind: "permission",
    id: "approve-calendar",
    permission: "mcp_tool",
    onDenied: "branch",
  },
  { kind: "output", id: "calendar-denied" },
  {
    kind: "mcp_tool",
    id: "calendar",
    serverName: "oomu_task_tools",
    toolName: "create_conflict_free_calendar_event",
  },
  {
    kind: "permission",
    id: "approve-send",
    permission: "mcp_tool",
    onDenied: "branch",
  },
  { kind: "output", id: "send-denied" },
  {
    kind: "mcp_tool",
    id: "send",
    serverName: "oomu_task_tools",
    toolName: "send_system_email",
  },
  { kind: "output", id: "output" },
];

export const scenarioSixWorkflowSteps = JSON.stringify({
  workflowIr: {
    nodes: scenarioSixNodes,
    edges: [
      ...serialEdges([
        "input",
        "read-suppliers",
        "analyze-suppliers",
        "source",
        "assess",
        "validate-report",
        "write-report",
        "has-exception",
      ]),
      {
        sourceNodeId: "has-exception",
        sourcePort: "false",
        targetNodeId: "no-exception",
      },
      {
        sourceNodeId: "has-exception",
        sourcePort: "true",
        targetNodeId: "approve-calendar",
      },
      {
        sourceNodeId: "approve-calendar",
        sourcePort: "denied",
        targetNodeId: "calendar-denied",
      },
      {
        sourceNodeId: "approve-calendar",
        sourcePort: "approved",
        targetNodeId: "calendar",
      },
      {
        sourceNodeId: "calendar",
        sourcePort: "out",
        targetNodeId: "approve-send",
      },
      {
        sourceNodeId: "approve-send",
        sourcePort: "denied",
        targetNodeId: "send-denied",
      },
      {
        sourceNodeId: "approve-send",
        sourcePort: "approved",
        targetNodeId: "send",
      },
      { sourceNodeId: "send", sourcePort: "out", targetNodeId: "output" },
    ],
  },
});
