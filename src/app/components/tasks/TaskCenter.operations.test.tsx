import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskCenter } from "./TaskCenter";
import { peekTaskFocus, requestTaskFocus } from "./taskFocus";

const mocks = vi.hoisted(() => ({
  control: vi.fn(),
  events: vi.fn(),
  list: vi.fn(),
  projects: vi.fn(),
  reconcile: vi.fn(),
  resolveEffectVerification: vi.fn(),
  resumeAgent: vi.fn(),
}));
const approvalsMock = vi.hoisted(() => ({
  value: null as null | {
    resolveWorkflowApproval: ReturnType<typeof vi.fn>;
    workflowApprovals: Array<Record<string, unknown>>;
  },
}));

const labels: Record<string, string> = {
  "approvals.approve": "Approve",
  "approvals.decline": "Decline",
  "common.loading": "Loading…",
  "chat.recovery.approval_save_draft_only": "Save one unsent draft",
  "permissions.unverified_action": "OOMU could not verify this action.",
  "tasks.activity": "Activity",
  "tasks.approval_error": "OOMU could not record your choice. The run is still paused.",
  "tasks.approval_help": "Review the step, then choose whether OOMU should continue.",
  "tasks.approval_recovering": "Getting the approval ready…",
  "tasks.approval_resolved": "Your choice was recorded.",
  "tasks.approval_title": "This run needs your OK",
  "tasks.all_projects": "All Projects",
  "tasks.control_error": "That task action didn't finish. Nothing else changed.",
  "tasks.control_retry": "Retry",
  "tasks.control_resume": "Resume",
  "tasks.control_success": "Task updated.",
  "tasks.empty": "Nothing running yet. Ask OOMU in Chat to start something.",
  "tasks.empty_title": "Nothing running yet.",
  "tasks.error_generic": "This task stopped before it finished.",
  "tasks.error_title": "What happened",
  "tasks.effect_verification_calendar_body": "This Calendar event may already have been created.",
  "tasks.effect_verification_calendar_inspect": "Open Calendar and look for this event.",
  "tasks.effect_verification_did_not_happen": "It didn’t happen — try this step once",
  "tasks.effect_verification_error": "That choice wasn’t saved. The action was not repeated.",
  "tasks.effect_verification_happened": "It happened — stop without repeating",
  "tasks.effect_verification_loading": "OOMU is loading the exact protected action…",
  "tasks.effect_verification_load_error": "OOMU couldn’t load the exact action details. The action was not repeated.",
  "tasks.effect_verification_reload": "Load details again",
  "tasks.effect_verification_retry_saved": "OOMU will try that exact step once.",
  "tasks.effect_verification_stop_saved": "OOMU stopped without repeating the action.",
  "tasks.effect_verification_stop_without_details": "Stop this task safely",
  "tasks.effect_verification_stop_without_repeating": "Stop this task without repeating",
  "tasks.effect_verification_title": "Check this action before OOMU continues",
  "tasks.effect_verification_working": "Saving your choice…",
  "tasks.filter_all": "All",
  "tasks.filter_awaiting_approval": "Needs you",
  "tasks.filter_blocked": "Blocked",
  "tasks.filter_cancelled": "Cancelled",
  "tasks.filter_completed": "Completed",
  "tasks.filter_failed": "Failed",
  "tasks.filter_history": "History",
  "tasks.filter_running": "Current",
  "tasks.go_to_chat": "Go to Chat",
  "tasks.load_error": "Tasks couldn't be loaded. Try checking again.",
  "tasks.no_events": "Nothing has happened here yet.",
  "tasks.origin_task": "Task",
  "tasks.origin_scheduled": "Scheduled",
  "tasks.project_filter": "Filter by Project",
  "tasks.now_help": "See what is running, waiting, finished, or needs your attention.",
  "tasks.now_title": "Everything OOMU is doing or has done for you.",
  "tasks.recovery_title": "Recovery status",
  "tasks.select": "Select a task to see the details.",
  "tasks.state_failed": "Failed",
  "tasks.state_completed": "Completed",
  "tasks.state_awaiting_approval": "Needs your OK",
  "tasks.supporting_details": "More from this run",
  "tasks.subtitle": "See and control everything OOMU is doing for you.",
  "tasks.title": "Tasks",
  "mcp_confirmation.action_create_calendar_event": "Create Calendar event",
  "mcp_confirmation.action_send_email": "Send email",
  "mcp_confirmation.arguments": "What OOMU will use",
  "mcp_confirmation.calendar": "Calendar",
  "mcp_confirmation.duration": "Duration",
  "mcp_confirmation.event_title": "Event",
  "mcp_confirmation.minutes": "minutes",
  "mcp_confirmation.next_weekday": "Next weekday",
  "mcp_confirmation.recipient": "To",
  "mcp_confirmation.server": "Connection",
  "mcp_confirmation.subject": "Subject",
  "mcp_confirmation.time_window": "Time window",
  "mcp_confirmation.tool": "Action",
};

vi.mock("@/context/I18nContext", () => {
  const t = (key: string, values?: Record<string, string | number>) => {
    if (key === "tasks.detail_meta") {
      return `${values?.state ?? ""} · ${values?.origin ?? ""}`;
    }
    return labels[key] ?? key;
  };
  return { useI18n: () => ({ t }) };
});
vi.mock("@/context/ApprovalContext", () => ({
  useOptionalApproval: () => approvalsMock.value,
}));
vi.mock("../projects/projectClient", () => ({
  projectApi: { list: mocks.projects },
}));
vi.mock("./taskClient", () => ({
  taskApi: {
    control: mocks.control,
    events: mocks.events,
    list: mocks.list,
    reconcile: mocks.reconcile,
    resolveEffectVerification: mocks.resolveEffectVerification,
    resumeAgent: mocks.resumeAgent,
  },
}));
vi.mock("../browser_automation/BrowserTaskPanel", () => ({ BrowserTaskPanel: () => null }));
vi.mock("../delegation/ChildWorkstreams", () => ({ ChildWorkstreams: () => null }));
vi.mock("./EvidenceTimeline", () => ({ EvidenceTimeline: () => null }));
vi.mock("../artifacts/review/CreateDocumentAction", () => ({ CreateDocumentAction: () => null }));
vi.mock("../media/MediaTaskPanel", () => ({ MediaTaskPanel: () => null }));
vi.mock("../learning/LearningReview", () => ({ LearningReview: () => null }));
vi.mock("../analysis/AnalysisResults", () => ({ AnalysisResults: () => null }));

const nativeCanary = "BACKEND CANARY: owning runtime command code 77";
const failedTask = {
  acknowledgedAtMs: null,
  completedAtMs: null,
  correlationId: "correlation-1",
  createdAtMs: 1,
  lastError: null,
  origin: "taskflow",
  projectId: null,
  recoveryState: "reconciled",
  effectVerificationRequired: false,
  runtimeKind: "taskflow",
  runtimeRecordId: "record-1",
  state: "failed",
  summary: "Prepare report",
  taskId: "task-1",
  taskRunId: "run-1",
  updatedAtMs: 2,
  validControls: ["retry"],
};

const awaitingWorkflowTask = {
  ...failedTask,
  lastError: null,
  origin: "routine",
  runtimeKind: "workflow",
  runtimeRecordId: "workflow-instance-1",
  state: "awaiting_approval",
  summary: "Send the weekly brief",
  taskId: "task-approval",
  taskRunId: "run-approval",
  validControls: [],
};

const completedScheduledTask = {
  ...failedTask,
  completedAtMs: 3,
  origin: "routine",
  recoveryState: "reconciled",
  runtimeKind: "workflow",
  runtimeRecordId: "workflow-instance-completed",
  state: "completed",
  summary: "Finished supplier brief",
  taskId: "task-completed",
  taskRunId: "run-completed",
  updatedAtMs: 3,
  validControls: [],
};

const protectedCalendarTask = {
  ...failedTask,
  effectVerificationRequired: true,
  lastError: "backend-only verification boundary",
  origin: "routine",
  recoveryState: "recoverable",
  runtimeKind: "workflow",
  runtimeRecordId: "workflow-instance-protected",
  state: "blocked",
  summary: "Create supplier review event",
  taskId: "task_11111111-1111-4111-8111-111111111111",
  taskRunId: "taskrun_22222222-2222-4222-8222-222222222222",
  validControls: [],
};

const protectedCalendarEvent = {
  correlationId: "correlation-protected",
  evidenceClass: "observed_result",
  eventType: "workflow.effect.verification_required",
  payload: {
    effectKind: "create_system_calendar_event",
    effectSummary: {
      calendarName: "OOMU Test",
      notes: "must never render",
      surface: "calendar",
      title: "Supplier Decision Review",
    },
    idempotencyKey: "effect-calendar-1",
    nextAction: "verify_only",
    nodeId: "calendar-node",
    retrySupported: true,
  },
  projectId: "project_33333333-3333-4333-8333-333333333333",
  schemaVersion: 1,
  sequence: 7,
  taskId: protectedCalendarTask.taskId,
  taskRunId: protectedCalendarTask.taskRunId,
  timestamp: "2026-07-21T12:00:00.000Z",
};

const workflowApproval = {
  instanceId: "workflow-instance-1",
  workflowId: "workflow-1",
  nodeId: "send-brief",
  message: "Send the weekly brief",
  context: {
    actionType: "workflow_permission",
    permissionKind: "network",
  },
  approvalToken: "approval-1",
  approveCommand: {},
  rejectCommand: {},
};

describe("TaskCenter operation errors", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
    approvalsMock.value = null;
    mocks.control.mockReset();
    mocks.events.mockReset().mockResolvedValue([]);
    mocks.list.mockReset().mockResolvedValue([]);
    mocks.projects.mockReset().mockResolvedValue([]);
    mocks.reconcile.mockReset().mockResolvedValue({
      inspected: 0,
      lost: 0,
      reconciled: 0,
      runtimeUnavailable: 0,
    });
    mocks.resolveEffectVerification.mockReset().mockResolvedValue({});
    mocks.resumeAgent.mockReset().mockResolvedValue({
      executionId: "execution-resume",
      planId: "plan-resume",
      sessionId: "session-resume",
    });
  });

  afterEach(cleanup);

  it("localizes task loading failures", async () => {
    mocks.list.mockRejectedValue(new Error(nativeCanary));
    render(<TaskCenter />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Tasks couldn't be loaded. Try checking again.",
    );
    expect(screen.queryByText(/BACKEND CANARY|owning runtime|code 77/i)).toBeNull();
  });

  it("keeps automatic reconciliation failures out of the user's way", async () => {
    mocks.reconcile.mockRejectedValue(new Error(nativeCanary));
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");

    expect(screen.queryByRole("alert")).toBeNull();
    expect(mocks.list).toHaveBeenCalledWith(undefined, "running");
    expect(screen.queryByText(/BACKEND CANARY|owning runtime|code 77/i)).toBeNull();
  });

  it("opens on Current and never renders a labeled polling control", async () => {
    const interval = vi.spyOn(window, "setInterval");
    render(<TaskCenter showIntroduction={false} />);

    expect(await screen.findByText("Everything OOMU is doing or has done for you.")).toBeVisible();
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    expect(mocks.list).toHaveBeenCalledWith(undefined, "running");
    expect(mocks.reconcile).toHaveBeenCalled();
    expect(interval).toHaveBeenCalledWith(expect.any(Function), 5_000);
    expect(screen.getByRole("button", { name: "Current" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: /refresh statuses/i })).toBeNull();
    expect(screen.queryByText(/process that owns|owning process/i)).toBeNull();
    interval.mockRestore();
  });

  it("refreshes silently when the window regains focus", async () => {
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    mocks.reconcile.mockClear();
    mocks.list.mockClear();

    fireEvent(window, new Event("focus"));

    await waitFor(() => expect(mocks.reconcile).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocks.list).toHaveBeenCalledWith(undefined, "running"));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("sends an empty user to the one place tasks begin", async () => {
    const onStartInChat = vi.fn();
    render(<TaskCenter onStartInChat={onStartInChat} />);
    fireEvent.click(await screen.findByRole("button", { name: "Go to Chat" }));

    expect(onStartInChat).toHaveBeenCalledTimes(1);
  });

  it("keeps completed states behind History", async () => {
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    expect(screen.queryByRole("button", { name: "Completed" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(screen.getByRole("button", { name: "Completed" })).toBeVisible();
    expect(screen.getByRole("button", { name: "All" })).toBeVisible();
  });

  it("reveals and selects a completed Routine result on its first Tasks load", async () => {
    requestTaskFocus(completedScheduledTask.taskRunId, completedScheduledTask.state);
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "completed" ? [completedScheduledTask] : [],
    );

    render(<TaskCenter />);

    expect(
      await screen.findByRole("heading", { name: "Finished supplier brief" }),
    ).toBeVisible();
    expect(mocks.list).toHaveBeenCalledWith(undefined, "completed");
    expect(screen.getByRole("button", { name: "History" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByRole("button", { name: "Completed" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(peekTaskFocus()).toBeNull();
  });

  it("localizes task-control failures", async () => {
    mocks.list.mockResolvedValue([failedTask]);
    mocks.control.mockRejectedValue(new Error(nativeCanary));
    render(<TaskCenter />);
    fireEvent.click(await screen.findByRole("button", { name: "Retry" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That task action didn't finish. Nothing else changed.",
    );
    expect(screen.queryByText(/BACKEND CANARY|owning runtime|code 77/i)).toBeNull();
  });

  it("resumes a blocked agent from its durable execution checkpoint", async () => {
    mocks.list.mockResolvedValue([{
      ...failedTask,
      origin: "chat",
      runtimeKind: "agent",
      runtimeRecordId: "execution-resume",
      state: "blocked",
      summary: "Prepare supplier decision pack",
      validControls: ["resume"],
    }]);
    render(<TaskCenter />);
    fireEvent.click(await screen.findByRole("button", { name: "Resume" }));

    await waitFor(() => expect(mocks.resumeAgent).toHaveBeenCalledWith("execution-resume"));
    expect(mocks.control).not.toHaveBeenCalled();
    expect(await screen.findByRole("status")).toHaveTextContent("Task updated.");
  });

  it("replaces generic Resume with an exact, non-looping protected-action decision", async () => {
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "blocked" ? [protectedCalendarTask] : [],
    );
    mocks.events.mockResolvedValue([protectedCalendarEvent]);
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Blocked" }));

    expect(await screen.findByRole("heading", {
      name: "Check this action before OOMU continues",
    })).toBeVisible();
    expect(await screen.findByText("OOMU Test")).toBeVisible();
    expect(screen.getByText("Calendar").nextSibling).toHaveTextContent("OOMU Test");
    expect(screen.getByText("Event").nextSibling).toHaveTextContent("Supplier Decision Review");
    expect(screen.queryByText("must never render")).toBeNull();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();

    fireEvent.click(screen.getByRole("button", {
      name: "It didn’t happen — try this step once",
    }));
    await waitFor(() => expect(mocks.resolveEffectVerification).toHaveBeenCalledWith({
      decision: "did_not_happen",
      effectKind: "create_system_calendar_event",
      idempotencyKey: "effect-calendar-1",
      nodeId: "calendar-node",
      runtimeRecordId: "workflow-instance-protected",
      taskId: protectedCalendarTask.taskId,
      taskRunId: protectedCalendarTask.taskRunId,
      verificationSequence: 7,
    }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "OOMU will try that exact step once.",
    );
  });

  it("offers a safe reload when exact protected-action details cannot be read", async () => {
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "blocked" ? [protectedCalendarTask] : [],
    );
    let eventAttempts = 0;
    mocks.events.mockImplementation(async () => {
      eventAttempts += 1;
      if (eventAttempts === 1) throw new Error(nativeCanary);
      return [protectedCalendarEvent];
    });
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Blocked" }));

    expect(await screen.findByText(
      "OOMU couldn’t load the exact action details. The action was not repeated.",
    )).toBeVisible();
    expect(screen.getByRole("button", { name: "Stop this task safely" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Load details again" }));
    await waitFor(() => expect(mocks.events).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("OOMU Test")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  });

  it("can stop safely without replay when protected-action details stay unavailable", async () => {
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "blocked" ? [protectedCalendarTask] : [],
    );
    mocks.events.mockRejectedValue(new Error(nativeCanary));
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Blocked" }));

    fireEvent.click(await screen.findByRole("button", {
      name: "Stop this task safely",
    }));
    await waitFor(() => expect(mocks.resolveEffectVerification).toHaveBeenCalledWith({
      decision: "stop_without_repeating",
      runtimeRecordId: "workflow-instance-protected",
      taskId: protectedCalendarTask.taskId,
      taskRunId: protectedCalendarTask.taskRunId,
    }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "OOMU stopped without repeating the action.",
    );
  });

  it("offers a neutral no-replay choice without changing the verified outcome", async () => {
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "blocked" ? [protectedCalendarTask] : [],
    );
    mocks.events.mockResolvedValue([protectedCalendarEvent]);
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Blocked" }));

    fireEvent.click(await screen.findByRole("button", {
      name: "Stop this task without repeating",
    }));
    await waitFor(() => expect(mocks.resolveEffectVerification).toHaveBeenCalledWith({
      decision: "stop_without_repeating",
      effectKind: "create_system_calendar_event",
      idempotencyKey: "effect-calendar-1",
      nodeId: "calendar-node",
      runtimeRecordId: "workflow-instance-protected",
      taskId: protectedCalendarTask.taskId,
      taskRunId: protectedCalendarTask.taskRunId,
      verificationSequence: 7,
    }));
  });

  it("puts a working Approve and Decline decision above secondary run details", async () => {
    const resolveWorkflowApproval = vi.fn().mockResolvedValue(true);
    approvalsMock.value = {
      resolveWorkflowApproval,
      workflowApprovals: [workflowApproval],
    };
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "awaiting_approval" ? [awaitingWorkflowTask] : [],
    );
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Needs you" }));

    const approvalTitle = await screen.findByRole("heading", { name: "This run needs your OK" });
    const activityTitle = screen.getByRole("heading", { name: "Activity" });
    expect(approvalTitle.compareDocumentPosition(activityTitle) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText("Needs your OK · Scheduled")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(resolveWorkflowApproval).toHaveBeenCalledWith(
      "workflow-instance-1",
      "approve",
    ));
    expect(await screen.findByRole("status")).toHaveTextContent("Your choice was recorded.");
  });

  it("keeps approval blocked for an unrecognized workflow action while allowing decline", async () => {
    approvalsMock.value = {
      resolveWorkflowApproval: vi.fn().mockResolvedValue(true),
      workflowApprovals: [{ ...workflowApproval, context: {} }],
    };
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "awaiting_approval" ? [awaitingWorkflowTask] : [],
    );

    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Needs you" }));

    await screen.findByText("OOMU could not verify this action.");
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Decline" })).toBeEnabled();
  });

  it("shows the exact recovered Calendar approval without exposing notes", async () => {
    approvalsMock.value = {
      resolveWorkflowApproval: vi.fn().mockResolvedValue(true),
      workflowApprovals: [{
        ...workflowApproval,
        context: {
          actionType: "mcp_tool",
          arguments: {
            availability: "tentative",
            calendarName: "OOMU Test",
            day: "next_weekday",
            durationMinutes: 30,
            notes: "private agenda details",
            title: "Supplier Decision Review",
            windowEndLocal: "16:00",
            windowStartLocal: "13:00",
          },
          serverName: "oomu_task_tools",
          toolName: "create_conflict_free_calendar_event",
        },
      }],
    };
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "awaiting_approval" ? [awaitingWorkflowTask] : [],
    );
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Needs you" }));

    expect(await screen.findByText("Create Calendar event")).toBeVisible();
    expect(screen.getByText("Calendar: OOMU Test")).toBeVisible();
    expect(screen.getByText("Event: Supplier Decision Review")).toBeVisible();
    expect(screen.queryByText("private agenda details")).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("shows recipient and subject for recovered Mail-draft approval but never the body", async () => {
    approvalsMock.value = {
      resolveWorkflowApproval: vi.fn().mockResolvedValue(true),
      workflowApprovals: [{
        ...workflowApproval,
        context: {
          actionType: "mcp_tool",
          arguments: {
            body: "confidential message body",
            subject: "Supplier decision",
            to: "recipient@example.com",
          },
          serverName: "oomu_task_tools",
          toolName: "draft_system_email",
        },
      }],
    };
    mocks.list.mockImplementation(async (_projectId?: string, state?: string) =>
      state === "awaiting_approval" ? [awaitingWorkflowTask] : [],
    );
    render(<TaskCenter />);
    await screen.findByText("Nothing running yet. Ask OOMU in Chat to start something.");
    fireEvent.click(screen.getByRole("button", { name: "Needs you" }));

    expect(await screen.findByText("Save one unsent draft")).toBeVisible();
    expect(screen.getByText("To: recipient@example.com")).toBeVisible();
    expect(screen.getByText("Subject: Supplier decision")).toBeVisible();
    expect(screen.queryByText("confidential message body")).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });
});
