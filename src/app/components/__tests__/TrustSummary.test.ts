import { describe, expect, it } from "vitest";
import {
  buildTrustSummaryModel,
  buildWorkflowStoryModel,
} from "../TrustSummary";
import {
  MEDIUM_TIMEOUT_MS,
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  type WorkflowIr,
} from "../workflowIr";

function agentNode(id: string, label: string, objective: string) {
  return {
    kind: "agent" as const,
    id,
    label,
    objective,
    inputMappings: {
      context: "{{workflow.input}}",
    },
    outputKey: `nodes.${id}.output`,
    systemTimeoutMs: MEDIUM_TIMEOUT_MS,
  };
}

describe("buildTrustSummaryModel", () => {
  it("deduplicates repeated action disclosures before limiting the list", () => {
    const workflowIr: WorkflowIr = {
      schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
      workflowId: "wf-trust-summary",
      workflowVersion: 1,
      name: "Trust summary fixture",
      description: "Exercises repeated agent trust copy.",
      compiler: {
        model: WORKFLOW_COMPILER_MODEL,
      },
      nodes: [
        {
          kind: "input",
          id: "input",
          label: "Input",
          outputKey: "workflow.input",
          inputSchema: {},
        },
        ...Array.from({ length: 6 }, (_, index) =>
          agentNode(
            `decision-agent-${index + 1}`,
            `Decision agent ${index + 1}`,
            "Recommend the next step.",
          ),
        ),
        agentNode("summary-agent", "Summarize", "Summarize what changed."),
        {
          kind: "output",
          id: "output",
          label: "Output",
          inputMapping: "{{nodes.summary-agent.output}}",
          outputSchema: {},
        },
      ],
      edges: [
        {
          id: "edge-input-agent",
          sourceNodeId: "input",
          sourcePort: "out",
          targetNodeId: "decision-agent-1",
        },
      ],
    };

    expect(buildTrustSummaryModel(workflowIr).actions).toEqual([
      "Summarizes what it found and suggests the next step.",
      "Summarizes what it found.",
    ]);
  });

  it("builds a human story without compiler input and output nodes", () => {
    const workflowIr: WorkflowIr = {
      schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
      workflowId: "wf-human-story",
      workflowVersion: 1,
      name: "Human story fixture",
      description: "Shows only the meaningful approval beat.",
      compiler: {
        model: WORKFLOW_COMPILER_MODEL,
      },
      nodes: [
        {
          kind: "input",
          id: "input",
          label: "Workflow Input",
          outputKey: "workflow.input",
          inputSchema: {},
        },
        {
          kind: "permission",
          id: "approval",
          label: "Ask first",
          permission: "file_write",
          reason: "saving the report.",
          onDenied: "fail",
        },
        {
          kind: "output",
          id: "output",
          label: "Workflow Output",
          inputMapping: "{{workflow.input}}",
          outputSchema: {},
        },
      ],
      edges: [
        {
          id: "edge-input-approval",
          sourceNodeId: "input",
          sourcePort: "out",
          targetNodeId: "approval",
        },
        {
          id: "edge-approval-output",
          sourceNodeId: "approval",
          sourcePort: "approved",
          targetNodeId: "output",
        },
      ],
    };
    const t = (key: string, variables?: Record<string, string | number>) => {
      const copy: Record<string, string> = {
        "workflows.storyboard.details.input": "Uses the request.",
        "workflows.storyboard.details.output": "Shows the result.",
        "workflows.storyboard.details.permission":
          "Pauses for your approval before {reason}",
        "workflows.storyboard.natures.act": "Act",
        "workflows.storyboard.natures.approve": "Ask first",
        "workflows.storyboard.natures.read": "Read",
        "workflows.storyboard.titles.input": "Start",
        "workflows.storyboard.titles.output": "Finish",
        "workflows.storyboard.titles.permission": "Ask first",
      };
      let value = copy[key] ?? key;
      Object.entries(variables ?? {}).forEach(([name, replacement]) => {
        value = value.split(`{${name}}`).join(String(replacement));
      });
      return value;
    };

    expect(buildWorkflowStoryModel(workflowIr, t).beats).toEqual([
      {
        detail: "Pauses for your approval before saving the report.",
        id: "approval",
        nature: "approve",
      },
    ]);
  });
});
