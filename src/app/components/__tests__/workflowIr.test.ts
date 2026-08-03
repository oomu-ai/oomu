import { describe, expect, it } from "vitest";
import {
  LEGACY_WORKFLOW_COMPILER_MODEL,
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  workflowIrSchema,
} from "../workflowIr";
import { workflowActionKindSchema } from "../workflowTypes";

type WorkflowIrFixture = {
  schemaVersion: string;
  workflowId: string;
  workflowVersion: number;
  name: string;
  description: string;
  compiler: { model: string };
  nodes: Record<string, unknown>[];
  edges: Record<string, unknown>[];
};

function validWorkflowIr(): WorkflowIrFixture {
  return {
    schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
    workflowId: "wf-test",
    workflowVersion: 1,
    name: "Test workflow",
    description: "Valid workflow fixture",
    compiler: {
      model: WORKFLOW_COMPILER_MODEL,
    },
    nodes: [
      {
        kind: "input",
        id: "wf-test:input",
        label: "Workflow Input",
        outputKey: "workflow.input",
        inputSchema: {
          type: "object",
        },
      },
      {
        kind: "agent",
        id: "agent-1",
        label: "Draft",
        objective: "Write a bounded draft.",
        inputMappings: {
          context: "{{workflow.input}}",
        },
        outputKey: "nodes.agent-1.output",
        systemTimeoutMs: 60000,
      },
      {
        kind: "output",
        id: "wf-test:output",
        label: "Workflow Output",
        inputMapping: "{{nodes.agent-1.output}}",
        outputSchema: {
          type: "object",
        },
      },
    ],
    edges: [
      {
        id: "edge-input-agent",
        sourceNodeId: "wf-test:input",
        sourcePort: "out",
        targetNodeId: "agent-1",
      },
      {
        id: "edge-agent-output",
        sourceNodeId: "agent-1",
        sourcePort: "out",
        targetNodeId: "wf-test:output",
      },
    ],
  };
}

describe("workflowIrSchema", () => {
  it("accepts a connected input-agent-output graph", () => {
    const parsed = workflowIrSchema.parse(validWorkflowIr());

    expect(parsed.workflowId).toBe("wf-test");
    expect(parsed.nodes.map((node) => node.kind)).toEqual([
      "input",
      "agent",
      "output",
    ]);
  });

  it("reads historical E2B workflow IR while emitting E4B by default", () => {
    const historical = validWorkflowIr();
    historical.compiler.model = LEGACY_WORKFLOW_COMPILER_MODEL;

    expect(workflowIrSchema.safeParse(historical).success).toBe(true);
    expect(validWorkflowIr().compiler.model).toBe(WORKFLOW_COMPILER_MODEL);

    const unknown = validWorkflowIr();
    unknown.compiler.model = "unknown-compiler";
    expect(workflowIrSchema.safeParse(unknown).success).toBe(false);
  });

  it("rejects duplicate edge ids", () => {
    const duplicateEdgeIr = validWorkflowIr();
    duplicateEdgeIr.edges[1] = {
      ...duplicateEdgeIr.edges[1],
      id: duplicateEdgeIr.edges[0].id,
    };

    const result = workflowIrSchema.safeParse(duplicateEdgeIr);

    expect(result.success).toBe(false);
    expect(result.error?.issues.some((issue) => issue.message === "Duplicate edge id: edge-input-agent")).toBe(true);
  });

  it("does not allow the removed hallucinated success action kind", () => {
    expect(workflowActionKindSchema.safeParse("hallucinated_success").success).toBe(false);
  });

  it("rejects null optional fields but accepts omitted ones (engine serialization contract)", () => {
    // The desktop engine emits IR through serde. Optional fields serialized as `null`
    // (instead of omitted) used to pass the Rust validator yet fail this schema's
    // `.optional()`/`.default()` checks, making every composed workflow unsaveable.
    // Lock both directions so the regression cannot return silently.
    expect(workflowIrSchema.safeParse(validWorkflowIr()).success).toBe(true);

    const nullTargetPort = validWorkflowIr();
    nullTargetPort.edges[0] = { ...nullTargetPort.edges[0], targetPort: null };
    expect(workflowIrSchema.safeParse(nullTargetPort).success).toBe(false);

    const nullTimeout = validWorkflowIr();
    nullTimeout.nodes[1] = { ...nullTimeout.nodes[1], systemTimeoutMs: null };
    expect(workflowIrSchema.safeParse(nullTimeout).success).toBe(false);
  });
});

describe("workflowIrSchema control-flow ports", () => {
  it("accepts conditional true and false ports", () => {
    const ir = validWorkflowIr();
    ir.nodes.splice(1, 1, {
      kind: "conditional",
      id: "condition-1",
      label: "If build passed",
      condition: "Did the build pass?",
      inputMapping: "{{workflow.input.data.status}}",
      systemTimeoutMs: 60000,
    });
    ir.edges = [
      {
        id: "edge-input-condition",
        sourceNodeId: "wf-test:input",
        sourcePort: "out",
        targetNodeId: "condition-1",
      },
      {
        id: "edge-condition-true",
        sourceNodeId: "condition-1",
        sourcePort: "true",
        targetNodeId: "wf-test:output",
      },
      {
        id: "edge-condition-false",
        sourceNodeId: "condition-1",
        sourcePort: "false",
        targetNodeId: "wf-test:output",
      },
    ];
    ir.nodes[2] = {
      ...ir.nodes[2],
      inputMapping: "{{workflow.output}}",
    };

    expect(workflowIrSchema.parse(ir).nodes[1]).toMatchObject({
      kind: "conditional",
      condition: "Did the build pass?",
    });
  });

  it("accepts loop item and done ports", () => {
    const ir = validWorkflowIr();
    ir.nodes.splice(1, 1,
      {
        kind: "loop",
        id: "loop-1",
        label: "For Each",
        itemsMapping: "{{workflow.input.data.files}}",
        itemVariable: "item",
        systemTimeoutMs: 60000,
      },
      {
        kind: "agent",
        id: "agent-1",
        label: "Summarize item",
        objective: "Summarize one item.",
        inputMappings: {
          context: "{{item}}",
        },
        outputKey: "nodes.agent-1.output",
        systemTimeoutMs: 60000,
      },
    );
    ir.edges = [
      {
        id: "edge-input-loop",
        sourceNodeId: "wf-test:input",
        sourcePort: "out",
        targetNodeId: "loop-1",
      },
      {
        id: "edge-loop-item",
        sourceNodeId: "loop-1",
        sourcePort: "item",
        targetNodeId: "agent-1",
      },
      {
        id: "edge-loop-done",
        sourceNodeId: "loop-1",
        sourcePort: "done",
        targetNodeId: "wf-test:output",
      },
      {
        id: "edge-agent-output",
        sourceNodeId: "agent-1",
        sourcePort: "out",
        targetNodeId: "wf-test:output",
      },
    ];
    ir.nodes[3] = {
      ...ir.nodes[3],
      inputMapping: "{{nodes.agent-1.output}}",
    };

    expect(workflowIrSchema.parse(ir).nodes.map((node) => node.kind)).toEqual([
      "input",
      "loop",
      "agent",
      "output",
    ]);
  });
});
