import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  loadSavedWorkflowIrs,
  loadSavedWorkflows,
  persistWorkflow,
  persistWorkflowIr,
  type SavedWorkflow,
} from "../workflowPersistence";
import {
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  workflowIrSchema,
  type WorkflowIr,
} from "../workflowIr";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: false,
}));

function fixtureIr(workflowId = "wf-persist"): WorkflowIr {
  return workflowIrSchema.parse({
    schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
    workflowId,
    workflowVersion: 1,
    name: "Persistence fixture",
    description: "Persist a storyboard-native workflow.",
    compiler: { model: WORKFLOW_COMPILER_MODEL },
    nodes: [
      {
        kind: "input",
        id: `${workflowId}:input`,
        label: "Workflow Input",
        outputKey: "workflow.input",
        inputSchema: { type: "object", additionalProperties: true },
      },
      {
        kind: "agent",
        id: "summarize",
        label: "Summarize",
        objective: "Summarize the input.",
        inputMappings: { context: "{{workflow.input}}" },
        outputKey: "nodes.summarize.output",
      },
      {
        kind: "output",
        id: `${workflowId}:output`,
        label: "Workflow Output",
        inputMapping: "{{nodes.summarize.output}}",
        outputSchema: { type: "object", additionalProperties: true },
      },
    ],
    edges: [
      {
        id: "edge-input-summarize",
        sourceNodeId: `${workflowId}:input`,
        sourcePort: "out",
        targetNodeId: "summarize",
      },
      {
        id: "edge-summarize-output",
        sourceNodeId: "summarize",
        sourcePort: "out",
        targetNodeId: `${workflowId}:output`,
      },
    ],
  });
}

function savedWorkflow(workflowIr = fixtureIr()): SavedWorkflow {
  return {
    id: workflowIr.workflowId,
    name: workflowIr.name,
    description: workflowIr.description,
    isActive: true,
    workflowIr,
    workflowVersion: workflowIr.workflowVersion,
    createdAt: 100,
    updatedAt: 200,
  };
}

describe("workflowPersistence", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("ignores legacy saved workflow rows that do not carry storyboard IR", async () => {
    invokeMock.mockResolvedValueOnce([
      {
        id: "wf-row",
        name: "Recovered Workflow",
        steps: "not-json",
        created_at: 10,
        updated_at: 20,
      },
    ]);

    const workflows = await loadSavedWorkflows();

    expect(invokeMock).toHaveBeenCalledWith("get_workflows");
    expect(workflows).toEqual([]);
  });

  it("loads IR-native workflow rows when blueprint IR exists", async () => {
    const workflowIr = fixtureIr("wf-row-ir");
    invokeMock.mockResolvedValueOnce([
      {
        workflowId: workflowIr.workflowId,
        version: 3,
        name: workflowIr.name,
        description: workflowIr.description,
        visualState: {
          description: workflowIr.description,
          isActive: true,
        },
        workflowIr: { ...workflowIr, workflowVersion: 3 },
        compilationStatus: "Compiled",
        projectId: "project-alpha",
        isActive: true,
        createdAtMs: 100,
        updatedAtMs: 300,
        compiledAtMs: 350,
      },
    ]);

    const records = await loadSavedWorkflowIrs();

    expect(invokeMock).toHaveBeenCalledWith("get_workflow_irs");
    expect(records).toHaveLength(1);
    expect(records[0].workflowIr.workflowVersion).toBe(3);
    expect(records[0].workflow.projectId).toBe("project-alpha");
  });

  it("persists reviewed editable steps", async () => {
    const workflow = { ...savedWorkflow(), projectId: "project-alpha" };
    invokeMock.mockResolvedValueOnce({
      workflowId: workflow.id,
      workflowVersion: 2,
      compilationStatus: "Compiled",
      compiledNodeCount: 1,
      projectId: "project-alpha",
      reviewCapabilities: {
        status: "ready",
        calendarCreate: false,
        calendarRead: false,
        emailDraft: false,
        emailRead: false,
        emailSend: false,
        officialWeb: false,
        projectFileRead: false,
        projectFileWrite: false,
      },
    });

    await persistWorkflowIr(workflow, workflow.workflowIr!);

    expect(invokeMock).toHaveBeenCalledWith(
      "save_workflow",
      expect.objectContaining({
        request: expect.objectContaining({
          projectId: "project-alpha",
          workflowIr: expect.objectContaining({ workflowId: workflow.id }),
          visualState: expect.objectContaining({
            projectId: "project-alpha",
            workflowIr: expect.objectContaining({ workflowId: workflow.id }),
          }),
        }),
      }),
    );
  });

  it("rejects legacy visual saves that do not carry editable steps", async () => {
    await expect(
      persistWorkflow({
        id: "wf-legacy",
        name: "Legacy",
        description: "No IR.",
        isActive: true,
        createdAt: 1,
        updatedAt: 1,
      } as SavedWorkflow),
    ).rejects.toThrow("editable steps");
  });
});
