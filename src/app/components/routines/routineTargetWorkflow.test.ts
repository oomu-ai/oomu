import { beforeEach, describe, expect, it, vi } from "vitest";
import { workflowIrSchema } from "../workflowIr";
import type { RoutineHandoffRequest } from "./routineDraft";
import {
  composeRoutineTargetWorkflow,
  materializeRoutineTargetWorkflow,
  plannedRoutineWorkflowAttachment,
} from "./routineTargetWorkflow";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));

const copy = {
  projectDescription: "Private Project scope for schedules created from Chat.",
  projectName: "Scheduled tasks",
  workflowDescription: "Runs the exact task requested in Chat.",
  workflowName: "Unread Mail Check",
};

function request(overrides: Partial<RoutineHandoffRequest> = {}): RoutineHandoffRequest {
  return {
    requestText: "Check my unread email every hour until midnight, then run it once.",
    scheduleText: "every 1 hour",
    scheduleKind: "recurring",
    cadence: { interval: 1, unit: "hour" },
    scheduleSupported: true,
    timingDefaulted: false,
    cadenceBoundaryConflict: false,
    runOnceRequested: true,
    endBoundary: "midnight",
    targetAction: { kind: "read_unread_mail" },
    ...overrides,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("Routine target workflow attachment", () => {
  it.each(["every 1 hour", "every day", "every week", "every month"])(
    "keeps cadence %s out of the exact unread-Mail workflow",
    (scheduleText) => {
      const attachment = plannedRoutineWorkflowAttachment(
        request({ scheduleText }),
        "draft-cadence",
        "project-1",
      );
      const tools = attachment.workflowIr?.nodes.filter(
        (node) => node.kind === "mcp_tool",
      );
      expect(tools).toEqual([
        expect.objectContaining({
          arguments: { max_messages: 20, unread_only: true },
          serverName: "macos_applescript",
          toolName: "read_system_emails",
        }),
      ]);
      expect(JSON.stringify(attachment.workflowIr)).not.toContain(scheduleText);
      expect(invokeMock).not.toHaveBeenCalled();
    },
  );

  it("sends an unmatched non-English task unchanged through the complete live catalog", async () => {
    const spanish = request({
      requestText: "Cada día, revisa mis notas y dime qué cambió.",
      scheduleText: "every day",
      targetAction: undefined,
    });
    const attachment = plannedRoutineWorkflowAttachment(
      spanish,
      "draft-spanish",
      "project-1",
    );
    const catalog = { authoringEnabled: true, generatedAtMs: 1, actions: [], templates: [], version: "v1" };
    const workflowIr = workflowIrSchema.parse({
      schemaVersion: "1.0.0",
      workflowId: attachment.workflowId,
      workflowVersion: 1,
      name: "Revisar notas",
      description: "",
      compiler: { model: "gemma-4-e4b-qat" },
      nodes: [
        { kind: "input", id: "input", label: "Input", outputKey: "workflow.input", inputSchema: { type: "object" } },
        { kind: "mcp_tool", id: "notes", label: "Read Notes", serverName: "macos_applescript", toolName: "read_system_notes", arguments: {} },
        { kind: "output", id: "output", label: "Output", inputMapping: "{{nodes.notes.output}}", outputSchema: { type: "object" } },
      ],
      edges: [
        { id: "e1", sourceNodeId: "input", sourcePort: "out", targetNodeId: "notes" },
        { id: "e2", sourceNodeId: "notes", sourcePort: "out", targetNodeId: "output" },
      ],
    });
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "get_workflow_capability_catalog") return catalog;
      if (command === "compose_workflow") {
        expect(args).toEqual({ request: expect.objectContaining({
          capabilityCatalog: catalog,
          prompt: spanish.requestText,
          projectId: "project-1",
          workflowId: attachment.workflowId,
        }) });
        return { status: "composed", reason: "", workflowIr, missingCapabilities: [], attempts: 1, latencyMs: 1 };
      }
      throw new Error(`Unexpected command: ${command}`);
    });

    const composed = await composeRoutineTargetWorkflow(spanish, attachment, copy);
    expect(composed.workflowIr?.nodes).toContainEqual(
      expect.objectContaining({ toolName: "read_system_notes" }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("save_workflow", expect.anything());
  });

  it("creates the planned Project and compiles the exact Mail workflow only on materialization", async () => {
    const attachment = plannedRoutineWorkflowAttachment(request(), "draft-global", null);
    invokeMock.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "list_projects" || command === "get_workflows") return [];
      if (command === "create_project") {
        expect(args?.request).toEqual({
          name: copy.projectName,
          description: copy.projectDescription,
          dataPolicy: "local_only",
        });
        return { projectId: "project-created", name: copy.projectName, description: copy.projectDescription, dataPolicy: "local_only" };
      }
      if (command === "save_workflow") {
        const requestValue = args?.request as Record<string, unknown>;
        expect(requestValue.projectId).toBe("project-created");
        expect(JSON.stringify(requestValue.workflowIr)).toContain("read_system_emails");
        expect(JSON.stringify(requestValue.workflowIr)).not.toContain("add_system_reminder");
        return {
          workflowId: attachment.workflowId,
          workflowVersion: 1,
          compilationStatus: "Compiled",
          compiledNodeCount: 5,
          projectId: "project-created",
          reviewCapabilities: {
            status: "ready", calendarCreate: false, calendarRead: false,
            emailDraft: false, emailRead: true, emailSend: false,
            officialWeb: false, projectFileRead: false, projectFileWrite: false,
          },
        };
      }
      throw new Error(`Unexpected command: ${command}`);
    });

    const attached = await materializeRoutineTargetWorkflow(attachment, copy);
    expect(attached).toMatchObject({
      projectId: "project-created",
      projectPlanned: false,
      workflowId: attachment.workflowId,
      workflowVersion: 1,
    });
  });
});
