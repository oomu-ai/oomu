import { createElement, type ReactNode } from "react";
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import {
  WORKFLOW_COMPILER_MODEL,
  WORKFLOW_IR_SCHEMA_VERSION,
  workflowIrSchema,
} from "../workflowIr";
import {
  friendlyWorkflowError,
  resolveFailedRunMessage,
  useWorkflowRun,
  workflowInputNodeId,
  workflowRunnableStepCount,
} from "../useWorkflowRun";
import type { SavedWorkflow, WorkflowRunResponse } from "../workflowPersistence";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: false,
}));

const localeState = {
  activeLocale: "en-US",
  availableLocales: [
    {
      fileName: "en-US.json",
      id: "en-US",
      isDefault: true,
      label: "English (US)",
      verified: true,
    },
  ],
  translations: {},
};

function I18nWrapper({ children }: { children: ReactNode }) {
  return createElement(I18nProvider, null, children);
}

const runnableWorkflowIr = workflowIrSchema.parse({
  schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
  workflowId: "wf-inbox-review",
  workflowVersion: 1,
  name: "Inbox Review",
  description: "Review unread messages.",
  compiler: { model: WORKFLOW_COMPILER_MODEL },
  nodes: [
    {
      kind: "input",
      id: "input",
      label: "Workflow Input",
      outputKey: "workflow.input",
      inputSchema: { type: "object" },
    },
    {
      kind: "mcp_tool",
      id: "read-mail",
      label: "Read unread mail",
      serverName: "mail",
      toolName: "read_messages",
      arguments: {},
      systemTimeoutMs: 10_000,
    },
    {
      kind: "output",
      id: "output",
      label: "Workflow Output",
      inputMapping: "{{nodes.read-mail.output}}",
      outputSchema: { type: "object" },
    },
  ],
  edges: [
    {
      id: "edge-input-read",
      sourceNodeId: "input",
      sourcePort: "out",
      targetNodeId: "read-mail",
    },
    {
      id: "edge-read-output",
      sourceNodeId: "read-mail",
      sourcePort: "out",
      targetNodeId: "output",
    },
  ],
});

const runnableWorkflow: SavedWorkflow = {
  id: "wf-inbox-review",
  name: "Inbox Review",
  description: "Review unread messages.",
  isActive: true,
  workflowIr: runnableWorkflowIr,
  workflowVersion: 1,
  createdAt: 1,
  updatedAt: 2,
};

function failedInstance(
  overrides: Partial<WorkflowRunResponse["instance"]>,
): WorkflowRunResponse["instance"] {
  return {
    id: "instance-1",
    workflowId: "wf-1",
    workflowVersion: 1,
    status: "Failed",
    nodePayloads: {},
    ...overrides,
  };
}

describe("useWorkflowRun helpers", () => {
  it("counts runnable steps from IR-native saved workflows", () => {
    const workflowIr = workflowIrSchema.parse({
      schemaVersion: WORKFLOW_IR_SCHEMA_VERSION,
      workflowId: "wf-ir-native",
      workflowVersion: 4,
      name: "IR native workflow",
      description: "Saved from the composer.",
      compiler: {
        model: WORKFLOW_COMPILER_MODEL,
      },
      nodes: [
        {
          kind: "input",
          id: "wf-ir-native:input",
          label: "Workflow Input",
          outputKey: "workflow.input",
          inputSchema: { type: "object" },
        },
        {
          kind: "agent",
          id: "agent-1",
          label: "Summarize",
          objective: "Summarize the input.",
          inputMappings: {
            context: "{{workflow.input}}",
          },
          outputKey: "nodes.agent-1.output",
        },
        {
          kind: "output",
          id: "wf-ir-native:output",
          label: "Workflow Output",
          inputMapping: "{{nodes.agent-1.output}}",
          outputSchema: { type: "object" },
        },
      ],
      edges: [
        {
          id: "edge-input-agent",
          sourceNodeId: "wf-ir-native:input",
          sourcePort: "out",
          targetNodeId: "agent-1",
        },
        {
          id: "edge-agent-output",
          sourceNodeId: "agent-1",
          sourcePort: "out",
          targetNodeId: "wf-ir-native:output",
        },
      ],
    });
    const workflow: SavedWorkflow = {
      id: "wf-ir-native",
      name: "IR native workflow",
      description: "Saved from the composer.",
      isActive: true,
      workflowIr,
      workflowVersion: 4,
      createdAt: 1,
      updatedAt: 2,
    };

    expect(workflowRunnableStepCount(workflow)).toBe(1);
    expect(workflowInputNodeId(workflow)).toBe("wf-ir-native:input");
  });
});

describe("useWorkflowRun completion messages", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("presents a typed empty collection as a clear successful no-op", async () => {
    const response: WorkflowRunResponse = {
      instance: {
        id: "instance-empty",
        workflowId: runnableWorkflow.id,
        workflowVersion: 1,
        status: "Completed",
        nodePayloads: {
          "read-mail": { status: "Completed", output: [] },
        },
      },
      executionOrder: ["input", "read-mail", "output"],
      completion: { kind: "empty_collection" },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_locale_state") return Promise.resolve(localeState);
      if (command === "run_workflow") return Promise.resolve(response);
      if (command === "update_workflow_last_run") return Promise.resolve(true);
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });

    const { result } = renderHook(() => useWorkflowRun(), {
      wrapper: I18nWrapper,
    });

    await act(async () => {
      await result.current.runWorkflow(runnableWorkflow);
    });

    expect(result.current.status).toBe(
      'Workflow "Inbox Review" completed. Nothing matched.',
    );
    expect(result.current.toast).toEqual({
      message: "You’re all caught up. Nothing matched this workflow.",
      tone: "success",
    });
    expect(result.current.lastRun?.completion).toEqual({
      kind: "empty_collection",
    });
    expect(invokeMock).toHaveBeenCalledWith(
      "update_workflow_last_run",
      expect.objectContaining({ id: runnableWorkflow.id }),
    );
  });
});

describe("resolveFailedRunMessage", () => {
  it("preserves registered tool schema failures instead of blaming workflow steps", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_registered_task_invalid",
        message: "fetch_official_page arguments do not match the registered schema.",
      }),
    ).toBe("fetch_official_page arguments do not match the registered schema.");
  });

  it("maps an actual workflow IR failure to workflow recovery guidance", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_ir_invalid",
        message: "Workflow IR contains an invalid edge.",
      }),
    ).toBe(
      "This workflow needs a valid storyboard before it can run. Review the steps, save again, and try once more.",
    );
  });

  it("keeps Apple-app transport details out of the user-facing error", () => {
    expect(
      friendlyWorkflowError(
        "MCP server 'macos_applescript' disconnected. A fresh one-use Shield Gate approval is required before reconnecting the non-native transport.",
      ),
    ).toBe("OOMU couldn't reach the Apple app this workflow needs. Try again.");
  });

  it("keeps unresolved workflow plumbing out of the user-facing error", () => {
    expect(
      friendlyWorkflowError(
        "Template reference nodes.read-unread-emails.output.emails.0.subject is unresolved.",
      ),
    ).toBe(
      "This workflow couldn't use the result from an earlier step. Nothing was changed. Try again.",
    );
  });

  it("turns a Calendar node deadline into a short recovery message", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_node_timeout",
        message:
          "Node Execution Timed Out: node calendar-assistant-read (Read macOS Calendar) exceeded 10000ms.",
      }),
    ).toBe("Calendar took too long to respond. Try again.");
  });

  it("keeps native Calendar permission recovery direct and specific", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_calendar_permission",
        message: "Calendar access is not authorized.",
      }),
    ).toBe(
      "Calendar access needs to be refreshed. Open System Settings, turn OOMU's Calendar access off and back on, then try again.",
    );
  });

  it("does not mislabel a native Calendar read failure as an Apple app outage", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_calendar_unavailable",
        message: "Calendar could not be read.",
      }),
    ).toBe("Calendar couldn't be read right now. Try again.");
  });

  it("keeps non-Calendar node deadlines free of technical details", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_node_timeout",
        message: "Node Execution Timed Out: node read-mail exceeded 10000ms.",
      }),
    ).toBe("This step took too long to respond. Try again.");
  });

  it("explains how to restore native notifications", () => {
    expect(
      friendlyWorkflowError({
        code: "workflow_runtime_notification_unavailable",
        message: "notification bridge failed",
      }),
    ).toBe(
      "Notifications are off for OOMU. Turn them on in System Settings, then try again.",
    );
  });

  it("surfaces the instance-level error for structural failures with no node payload", () => {
    // The runtime sets instance.error (not a node payload) when a workflow fails
    // before/around node execution, e.g. "No reachable Output node completed.".
    // This is exactly the case that used to collapse to "Unknown execution error.".
    const message = resolveFailedRunMessage(
      failedInstance({
        error: {
          code: "workflow_execution_error",
          message: "No reachable Output node completed.",
        },
      }),
    );
    expect(message).toBe("No reachable Output node completed.");
  });

  it("prefers the node-specific error when a step fails", () => {
    const message = resolveFailedRunMessage(
      failedInstance({
        error: { code: "x", message: "generic instance error" },
        nodePayloads: {
          "agent-1": {
            status: "Failed",
            error: {
              code: "agent_failed",
              boundary: "model",
              message: "The agent step could not complete.",
            },
          },
        },
      }),
    );
    expect(message).toBe("The agent step could not complete.");
  });

  it("falls back to the generic message only when no error detail exists", () => {
    expect(resolveFailedRunMessage(failedInstance({}))).toBe(
      "Unknown execution error.",
    );
  });
});
