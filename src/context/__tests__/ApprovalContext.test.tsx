import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useEffect, useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { McpConfirmationModal } from "@/components/mcp/McpConfirmationModal";
import {
  ApprovalProvider,
  useApproval,
  useWorkflowApprovalClaim,
} from "@/context/ApprovalContext";
import { I18nProvider } from "@/context/I18nContext";
import type { ApprovalResult } from "@/lib/approvalContracts";

const invokeMock = vi.hoisted(() => vi.fn());
const listeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (eventName: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(eventName, handler);
    return () => listeners.delete(eventName);
  }),
}));

function Providers({ children }: { children: React.ReactNode }) {
  return (
    <I18nProvider>
      <ApprovalProvider>{children}</ApprovalProvider>
    </I18nProvider>
  );
}

function LocalQueueProbe({ onResult }: { onResult: (result: ApprovalResult) => void }) {
  const approvals = useApproval();
  const requestedRef = useRef(false);
  useEffect(() => {
    if (requestedRef.current) return;
    requestedRef.current = true;
    for (const token of ["local-a", "local-b"]) {
      void approvals.requestApproval({
        approvalToken: token,
        actionType: "mcp_tool_call",
        actionLabel: `tool-${token}`,
        riskTier: "high",
        reason: "raw backend reason",
        requestedAtMs: Date.now(),
        preview: '{"raw":"json"}',
        approvalScopeKinds: ["once"],
      }).then(onResult);
    }
  }, [approvals, onResult]);
  return null;
}

function CancelSessionProbe({ sessionId }: { sessionId: string }) {
  const approvals = useApproval();
  return (
    <button
      onClick={() => void approvals.cancelApprovalsForSession(sessionId)}
      type="button"
    >
      Stop session
    </button>
  );
}

function ApprovalIndicatorProbe() {
  const approvals = useApproval();
  return (
    <button onClick={approvals.focusNextApproval} type="button">
      Review approvals ({approvals.pendingApprovalCount})
    </button>
  );
}

function LiveWorkflowApprovalProbe({ instanceId }: { instanceId: string }) {
  useWorkflowApprovalClaim(instanceId);
  return (
    <McpConfirmationModal
      argumentsValue={{}}
      isOpen
      onApprove={vi.fn()}
      onCancel={vi.fn()}
      serverName="Local tools"
      toolName="Read folder"
    />
  );
}

const workflowApproval = {
  instanceId: "workflow-instance-1",
  workflowId: "workflow-1",
  nodeId: "review-step",
  message: "Review the report before OOMU sends it.",
  context: {
    actionType: "workflow_permission",
    permissionKind: "network",
  },
  approvalToken: "workflow-approval-1",
  approveCommand: {},
  rejectCommand: {},
};

describe("ApprovalProvider", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listeners.clear();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return {
          activeLocale: "en-US",
          availableLocales: [],
          translations: {},
        };
      }
      if (command === "list_pending_shield_approvals") return [];
      if (command === "list_pending_workflow_approvals") return [];
      if (command === "resolve_shield_approval") return { status: "resolved" };
      return null;
    });
  });
  afterEach(cleanup);

  it("surfaces a detached workflow approval globally and resolves it through the workflow lane", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return { activeLocale: "en-US", availableLocales: [], translations: {} };
      }
      if (command === "list_pending_shield_approvals") return [];
      if (command === "list_pending_workflow_approvals") return [];
      if (command === "resolve_workflow_permission") {
        return { instance: { status: "Running" }, executionOrder: [] };
      }
      return null;
    });

    render(<div>Current screen</div>, { wrapper: Providers });
    await waitFor(() => expect(listeners.has("workflow://approval-requested")).toBe(true));
    act(() => listeners.get("workflow://approval-requested")?.({ payload: workflowApproval }));

    expect(await screen.findByRole("heading", { name: "Approve this step?" })).toBeVisible();
    expect(screen.getByText("OOMU needs your OK before it continues.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resolve_workflow_permission",
      {
        request: {
          approvalToken: "workflow-approval-1",
          decision: "approve",
          instanceId: "workflow-instance-1",
        },
      },
    ));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    const declinedApproval = {
      ...workflowApproval,
      approvalToken: "workflow-approval-2",
      instanceId: "workflow-instance-2",
    };
    act(() => listeners.get("workflow://approval-requested")?.({ payload: declinedApproval }));
    fireEvent.click(await screen.findByRole("button", { name: "Decline" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "resolve_workflow_permission",
      {
        request: {
          approvalToken: "workflow-approval-2",
          decision: "reject",
          instanceId: "workflow-instance-2",
        },
      },
    ));
  });

  it("shows exact consequential fields before a detached workflow approval", async () => {
    render(<div>Current screen</div>, { wrapper: Providers });
    await waitFor(() => expect(listeners.has("workflow://approval-requested")).toBe(true));
    act(() => listeners.get("workflow://approval-requested")?.({
      payload: {
        ...workflowApproval,
        context: {
          actionType: "mcp_tool",
          serverName: "oomu_task_tools",
          toolName: "send_system_email",
          arguments: {
            to: "recipient@example.com",
            subject: "OOMU Test — Supplier Exception",
            body: "This body must stay out of the approval preview.",
          },
        },
      },
    }));

    const exactFields = await screen.findByRole("region", {
      name: "What OOMU will use",
    });
    expect(exactFields).toHaveTextContent("To: recipient@example.com");
    expect(exactFields).toHaveTextContent(
      "Subject: OOMU Test — Supplier Exception",
    );
    expect(screen.getByText("OOMU")).toBeVisible();
    expect(screen.queryByText(/This body must stay out/)).toBeNull();
  });

  it("explains when one review will cover later runs of the unchanged workflow", async () => {
    render(<div>Current screen</div>, { wrapper: Providers });
    await waitFor(() => expect(listeners.has("workflow://approval-requested")).toBe(true));
    act(() => listeners.get("workflow://approval-requested")?.({
      payload: {
        ...workflowApproval,
        context: {
          actionType: "mcp_tool",
          serverName: "oomu_task_tools",
          toolName: "fetch_official_page",
          arguments: { url: "https://www.eia.gov/petroleum/gasdiesel/" },
          approvalReuse: {
            scope: "workflow_version",
            workflowVersion: 1,
          },
        },
      },
    }));

    expect(await screen.findByRole("button", {
      name: "Approve for this workflow",
    })).toBeVisible();
    expect(screen.getByText(
      "Approve this saved workflow once. OOMU asks again if the workflow, tool, or destination changes.",
    )).toBeVisible();
  });

  it("recovers pending workflow approvals and reopens a dismissed prompt from persistent state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return { activeLocale: "en-US", availableLocales: [], translations: {} };
      }
      if (command === "list_pending_shield_approvals") return [];
      if (command === "list_pending_workflow_approvals") return [workflowApproval];
      return null;
    });

    render(<ApprovalIndicatorProbe />, { wrapper: Providers });
    const dialog = await screen.findByRole("dialog");
    expect(screen.getByRole("button", { name: "Review approvals (1)" })).toBeVisible();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Review approvals (1)" }));
    expect(await screen.findByRole("heading", { name: "Approve this step?" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(
      screen.getByRole("button", { name: "Review approvals (0)" }),
    ).toBeVisible());
  });

  it("lets an active Composer claim its workflow approval without a second global prompt", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return { activeLocale: "en-US", availableLocales: [], translations: {} };
      }
      if (command === "list_pending_shield_approvals") return [];
      if (command === "list_pending_workflow_approvals") return [];
      return null;
    });

    render(<LiveWorkflowApprovalProbe instanceId="workflow-instance-1" />, {
      wrapper: Providers,
    });
    expect(await screen.findByRole("heading", { name: "Approve protected action" })).toBeVisible();
    await waitFor(() => expect(listeners.has("workflow://approval-requested")).toBe(true));
    act(() => listeners.get("workflow://approval-requested")?.({ payload: workflowApproval }));
    await act(async () => new Promise((resolve) => window.setTimeout(resolve, 0)));

    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.queryByRole("heading", { name: "Approve this step?" })).toBeNull();
  });

  it("queues simultaneous local approvals instead of dropping the second request", async () => {
    const results: ApprovalResult[] = [];
    render(<LocalQueueProbe onResult={(result) => results.push(result)} />, {
      wrapper: Providers,
    });

    expect(await screen.findByRole("dialog")).toBeVisible();
    const approve = screen.getByRole("button", { name: "Approve" });
    fireEvent.click(approve);
    fireEvent.click(approve);
    await waitFor(() => expect(results).toHaveLength(1));
    expect(screen.getByRole("dialog")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    await waitFor(() => expect(results).toEqual([
      { decision: "approve", scopeKind: "once" },
      { decision: "deny", scopeKind: "once" },
    ]));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("hydrates native Shield status without exposing an authorization token", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return { activeLocale: "en-US", availableLocales: [], translations: {} };
      }
      if (command === "list_pending_shield_approvals") {
        return [{
          displayId: "native-folder",
          sessionId: "session-1",
          actionLabel: "Read Research folder",
          semanticSummary: "OOMU wants to inspect files in the selected folder.",
          requestedAtMs: Date.now(),
          pending: true,
        }];
      }
      return null;
    });

    render(<div>Current screen</div>, { wrapper: Providers });
    expect(await screen.findByRole("heading", { name: "Review the native OOMU prompt" })).toBeVisible();
    expect(screen.getByText("Read Research folder")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Allow|Approve|Deny/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([command]) => command === "resolve_shield_approval")).toBe(false);
  });

  it("adds and removes the read-only status projection from native events", async () => {
    render(<div>Current screen</div>, { wrapper: Providers });
    await waitFor(() => expect(listeners.has("shield-approval-status-changed")).toBe(true));

    const status = {
      displayId: "display-only",
      sessionId: "session-1",
      actionLabel: "Run command",
      semanticSummary: "OOMU is waiting for your decision in the macOS prompt.",
      requestedAtMs: 1,
      pending: true,
    };
    act(() => listeners.get("shield-approval-status-changed")?.({ payload: status }));
    expect(await screen.findByRole("heading", { name: "Review the native OOMU prompt" })).toBeVisible();

    act(() => listeners.get("shield-approval-status-changed")?.({
      payload: { ...status, pending: false },
    }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("cancelling a session clears its native status without resolving authority", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_i18n_state") {
        return { activeLocale: "en-US", availableLocales: [], translations: {} };
      }
      if (command === "list_pending_shield_approvals") {
        return [{
          displayId: "cancel-native",
          sessionId: "session-to-stop",
          actionLabel: "Run command",
          semanticSummary: "Waiting for the macOS decision.",
          requestedAtMs: 1,
          pending: true,
        }];
      }
      return null;
    });

    render(<CancelSessionProbe sessionId="session-to-stop" />, { wrapper: Providers });
    expect(await screen.findByRole("dialog")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Stop session" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([command]) => command === "resolve_shield_approval")).toBe(false);
  });
});
