import { describe, expect, it } from "vitest";
import {
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  workflowIrSchema,
  type WorkflowIr,
} from "../workflowIr";
import {
  addConditionalBranch,
  buildWorkflowStoryboardModel,
} from "../WorkflowStoryboard";
import { buildWorkflowStoryModel } from "../TrustSummary";
import { firstSentenceForWorkflowPreview } from "../workflowPreviewText";

const translations: Record<string, string> = {
  "workflows.storyboard.branch_false": "Otherwise",
  "workflows.storyboard.branch_target": "Then: {target}",
  "workflows.storyboard.branch_true": "If the condition matches",
  "workflows.storyboard.details.input": "Captures run input as {key}.",
  "workflows.storyboard.details.conditional": "Continues only when: {condition}",
  "workflows.storyboard.details.output": "Returns {mapping}.",
  "workflows.storyboard.details.permission": "Pauses for approval before {reason}",
  "workflows.storyboard.kinds.agent": "Agent",
  "workflows.storyboard.kinds.conditional": "Decision",
  "workflows.storyboard.kinds.input": "Input",
  "workflows.storyboard.kinds.output": "Output",
  "workflows.storyboard.natures.act": "Act",
  "workflows.storyboard.natures.approve": "Ask first",
  "workflows.storyboard.natures.read": "Read",
  "workflows.storyboard.natures.think": "Think",
  "workflows.storyboard.titles.agent_draft": "Draft a message",
  "workflows.storyboard.titles.output": "Return the result",
  "workflows.storyboard.titles.permission": "Ask first",
  "workflows.trust.actions.deliver_configured_channel":
    "Deliver the verified result and exact filename.",
};

function t(key: string, variables?: Record<string, string | number>) {
  let value = translations[key] ?? key;
  Object.entries(variables ?? {}).forEach(([name, replacement]) => {
    value = value.split(`{${name}}`).join(String(replacement));
  });
  return value;
}

function branchingIr(): WorkflowIr {
  return workflowIrSchema.parse({
    schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
    workflowId: "wf-storyboard",
    workflowVersion: 1,
    name: "Storyboard fixture",
    description: "Exercises honest forks.",
    compiler: {
      model: WORKFLOW_COMPILER_MODEL,
    },
    nodes: [
      {
        kind: "input",
        id: "wf-storyboard:input",
        label: "Workflow Input",
        outputKey: "workflow.input",
        inputSchema: { type: "object" },
      },
      {
        kind: "conditional",
        id: "condition-1",
        label: "Check urgency",
        condition: "The request is urgent.",
        inputMapping: "{{workflow.input}}",
      },
      {
        kind: "agent",
        id: "draft-1",
        label: "Urgent Draft",
        objective: "Draft the urgent response.",
        inputMappings: {
          context: "{{workflow.input}}",
        },
        outputKey: "nodes.draft-1.output",
      },
      {
        kind: "output",
        id: "wf-storyboard:output",
        label: "Workflow Output",
        inputMapping: "{{workflow.output}}",
        outputSchema: { type: "object" },
      },
    ],
    edges: [
      {
        id: "edge-input-condition",
        sourceNodeId: "wf-storyboard:input",
        sourcePort: "out",
        targetNodeId: "condition-1",
      },
      {
        id: "edge-condition-true",
        sourceNodeId: "condition-1",
        sourcePort: "true",
        targetNodeId: "draft-1",
      },
      {
        id: "edge-condition-false",
        sourceNodeId: "condition-1",
        sourcePort: "false",
        targetNodeId: "wf-storyboard:output",
      },
      {
        id: "edge-draft-output",
        sourceNodeId: "draft-1",
        sourcePort: "out",
        targetNodeId: "wf-storyboard:output",
      },
    ],
  });
}

describe("WorkflowStoryboard", () => {
  it("renders conditional forks from the underlying IR edges", () => {
    const model = buildWorkflowStoryboardModel(branchingIr(), t);
    const condition = model.find((item) => item.id === "condition-1");

    expect(model.map((item) => item.id)).toEqual([
      "wf-storyboard:input",
      "condition-1",
      "draft-1",
      "wf-storyboard:output",
    ]);
    expect(condition?.nature).toBe("think");
    expect(condition).not.toHaveProperty("kindLabel");
    expect(condition?.branches).toEqual([
      expect.objectContaining({
        label: "If the condition matches",
        port: "true",
        targetId: "draft-1",
        targetLabel: "Draft a message",
      }),
      expect.objectContaining({
        label: "Otherwise",
        port: "false",
        targetId: "wf-storyboard:output",
        targetLabel: "Return the result",
      }),
    ]);
  });

  it("adds a branch as real conditional IR ports", () => {
    const ir = workflowIrSchema.parse({
      schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
      workflowId: "wf-linear",
      workflowVersion: 1,
      name: "Linear fixture",
      description: "Adds a branch.",
      compiler: {
        model: WORKFLOW_COMPILER_MODEL,
      },
      nodes: [
        {
          kind: "input",
          id: "wf-linear:input",
          label: "Workflow Input",
          outputKey: "workflow.input",
          inputSchema: { type: "object" },
        },
        {
          kind: "agent",
          id: "draft-1",
          label: "Draft",
          objective: "Draft a response.",
          inputMappings: {
            context: "{{workflow.input}}",
          },
          outputKey: "nodes.draft-1.output",
        },
        {
          kind: "output",
          id: "wf-linear:output",
          label: "Workflow Output",
          inputMapping: "{{nodes.draft-1.output}}",
          outputSchema: { type: "object" },
        },
      ],
      edges: [
        {
          id: "edge-input-draft",
          sourceNodeId: "wf-linear:input",
          sourcePort: "out",
          targetNodeId: "draft-1",
        },
        {
          id: "edge-draft-output",
          sourceNodeId: "draft-1",
          sourcePort: "out",
          targetNodeId: "wf-linear:output",
        },
      ],
    });

    const branched = addConditionalBranch(
      ir,
      "draft-1",
      "Only continue on weekdays.",
    );

    const conditional = branched.nodes.find(
      (node) => node.kind === "conditional",
    );
    expect(conditional).toMatchObject({
      condition: "Only continue on weekdays.",
    });
    expect(
      branched.edges.filter((edge) => edge.sourceNodeId === conditional?.id),
    ).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ sourcePort: "true" }),
        expect.objectContaining({ sourcePort: "false" }),
      ]),
    );
  });

  it("keeps JSONPath and email addresses whole in the visible review", () => {
    expect(firstSentenceForWorkflowPreview("$.hasException == true")).toBe(
      "$.hasException == true",
    );
    expect(
      firstSentenceForWorkflowPreview(
        "OOMU Test — Supplier Exception · recipient@example.com.",
      ),
    ).toBe("OOMU Test — Supplier Exception · recipient@example.com.");

    const workflow = workflowIrSchema.parse({
      schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
      workflowId: "wf-exact-preview",
      workflowVersion: 1,
      name: "Exact preview",
      description: "Keeps exact technical bindings visible.",
      compiler: { model: WORKFLOW_COMPILER_MODEL },
      metadata: { oomuRoutineDelivery: "configured_private_channel" },
      nodes: [
        {
          kind: "input",
          id: "input",
          label: "Input",
          outputKey: "workflow.input",
          inputSchema: { type: "object" },
        },
        {
          kind: "conditional",
          id: "has-exception",
          label: "Supplier exception",
          condition: "$.hasException == true",
          inputMapping: "{{nodes.analyze.output.data}}",
        },
        {
          kind: "output",
          id: "no-exception",
          label: "No exception",
          inputMapping: "{{nodes.report.output}}",
          outputSchema: { type: "object" },
        },
        {
          kind: "permission",
          id: "approve-send",
          label: "Approve send",
          permission: "mcp_tool",
          reason: "OOMU Test — Supplier Exception · recipient@example.com.",
          onDenied: "branch",
        },
        {
          kind: "output",
          id: "send-denied",
          label: "Send declined",
          inputMapping: "{{nodes.report.output}}",
          outputSchema: { type: "object" },
        },
        {
          kind: "mcp_tool",
          id: "send",
          label: "Send",
          serverName: "oomu_task_tools",
          toolName: "send_system_email",
          arguments: {
            to: "recipient@example.com",
            subject: "OOMU Test — Supplier Exception",
          },
        },
        {
          kind: "output",
          id: "output",
          label: "Delivered result",
          inputMapping: "{{nodes.send.output}}",
          outputSchema: { type: "object" },
        },
      ],
      edges: [
        {
          id: "e1",
          sourceNodeId: "input",
          sourcePort: "out",
          targetNodeId: "has-exception",
        },
        {
          id: "e2",
          sourceNodeId: "has-exception",
          sourcePort: "false",
          targetNodeId: "no-exception",
        },
        {
          id: "e3",
          sourceNodeId: "has-exception",
          sourcePort: "true",
          targetNodeId: "approve-send",
        },
        {
          id: "e4",
          sourceNodeId: "approve-send",
          sourcePort: "denied",
          targetNodeId: "send-denied",
        },
        {
          id: "e5",
          sourceNodeId: "approve-send",
          sourcePort: "approved",
          targetNodeId: "send",
        },
        {
          id: "e6",
          sourceNodeId: "send",
          sourcePort: "out",
          targetNodeId: "output",
        },
      ],
    });
    const storyboard = buildWorkflowStoryboardModel(workflow, t);

    expect(storyboard.find((item) => item.id === "has-exception")?.detail).toBe(
      "Continues only when: $.hasException == true",
    );
    expect(storyboard.find((item) => item.id === "approve-send")?.detail).toBe(
      "Pauses for approval before OOMU Test — Supplier Exception · recipient@example.com.",
    );

    const story = buildWorkflowStoryModel(workflow, t);
    expect(story.beats.find((beat) => beat.id === "output")?.detail).toBe(
      "Deliver the verified result and exact filename.",
    );
    expect(story.beats.some((beat) => beat.id === "no-exception")).toBe(false);
    expect(story.beats.some((beat) => beat.id === "send-denied")).toBe(false);
  });
});
