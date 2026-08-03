import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import {
  isCalendarPermissionRecoveryCode,
  isMailAutomationRecoveryCode,
  parseAgentExecutionRecoveryReceipt,
  RecoveryReceiptCard,
  resumablePermissionCapability,
} from "./RecoveryReceiptCard";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

function receipt(overrides: Record<string, unknown> = {}) {
  return JSON.stringify({
    schema: "oomu.agent_execution_recovery.v1",
    executionId: "agent-exec-test-6",
    planId: "plan-test-6",
    code: "decision_pack_research_evidence_unavailable",
    boundary: "DecisionPack",
    recoverable: true,
    message: "No current official freight source qualified.",
    context: {
      subject: "freight",
      attemptCount: 3,
      pageCount: 7,
      verifiedInputCount: 2,
    },
    changedState: "none",
    ...overrides,
  });
}

beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({
      activeLocale: "en-US",
      availableLocales: [{
        id: "en-US",
        label: "English (US)",
        fileName: "en-US.json",
        isDefault: true,
        verified: true,
      }],
      translations: {},
    });
});

afterEach(cleanup);

describe("RecoveryReceiptCard parsing and approval recovery", () => {

  it("accepts only the complete exact v1 contract and sanitizes context", () => {
    expect(parseAgentExecutionRecoveryReceipt(receipt())).toMatchObject({
      executionId: "agent-exec-test-6",
      planId: "plan-test-6",
      code: "decision_pack_research_evidence_unavailable",
      boundary: "DecisionPack",
      recoverable: true,
      recoveryAction: "resume_same_execution",
      message: "No current official freight source qualified.",
      changedState: "none",
      context: {
        subject: "freight",
        attemptCount: 3,
        pageCount: 7,
        verifiedInputs: 2,
        requestedCalendarName: null,
        availableCalendarNames: [],
        nextOperation: null,
        frozenArgumentSha256: null,
      },
    });

    const untrustedContext = receipt({
      context: {
        subject: "<script>alert(1)</script>",
        attemptCount: "three",
        pageCount: 1_000_000,
        verifiedInputCount: -1,
        rawError: "/Users/private/internal",
      },
    });
    expect(parseAgentExecutionRecoveryReceipt(untrustedContext)?.context).toEqual({
      subject: null,
      attemptCount: null,
      pageCount: null,
      verifiedInputs: null,
      requestedCalendarName: null,
      availableCalendarNames: [],
      nextOperation: null,
      frozenArgumentSha256: null,
      capabilityId: null,
    });
    const bounded = parseAgentExecutionRecoveryReceipt(receipt({
      message: `  ${"context ".repeat(80)}  `,
    }));
    expect(bounded?.message.length).toBeLessThanOrEqual(360);
    expect(bounded?.message.endsWith("…")).toBe(true);
  });

  it("derives a durable permission resume only from the matching recoverable execution", () => {
    const permissionReceipt = receipt({
      code: "calendar_permission_denied",
      changedState: "checkpoint_saved",
      recoveryAction: "resume_same_execution",
      context: { capabilityId: "calendar" },
    });
    expect(resumablePermissionCapability(permissionReceipt, "agent-exec-test-6")).toBe("calendar");
    expect(resumablePermissionCapability(permissionReceipt, "another-execution")).toBeNull();
    expect(resumablePermissionCapability(receipt({ recoverable: false, changedState: "none", recoveryAction: "start_new_plan" }), "agent-exec-test-6")).toBeNull();
  });

  it.each([
    { schema: "oomu.agent-execution-recovery.v1" },
    { executionId: "" },
    { planId: null },
    { boundary: "Decision Pack / raw" },
    { message: "" },
    { recoverable: "yes" },
    { changedState: "unknown" },
    { recoveryAction: "unsafe_replay" },
    { recoverable: false, recoveryAction: "resume_same_execution" },
  ])("rejects malformed privileged receipts: %o", (override) => {
    expect(parseAgentExecutionRecoveryReceipt(receipt(override))).toBeNull();
  });

  it("renders a localized research recovery and retries the exact execution", async () => {
    const onRetry = vi.fn(async () => undefined);
    render(<RecoveryReceiptCard content={receipt()} onRetry={onRetry} />, {
      wrapper: I18nProvider,
    });

    expect(screen.getByRole("region", { name: "freight research needs another pass" })).toBeVisible();
    expect(screen.getByText(/Verified source files: 2/)).toBeVisible();
    expect(screen.getByText("Nothing new was created.")).toBeVisible();
    expect(screen.queryByText(/decision_pack_research|agent-exec-test-6/)).toBeNull();

    fireEvent.click(screen.getByText("Review details"));
    expect(screen.getByText("Decision pack research")).toBeVisible();
    expect(screen.getByText("What needs attention")).toBeVisible();
    expect(screen.getByText("No current official freight source qualified.")).toBeVisible();
    expect(screen.getByText("Search attempts")).toBeVisible();
    expect(screen.getByText("Pages checked")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Retry research" }));
    await waitFor(() => expect(onRetry).toHaveBeenCalledWith("agent-exec-test-6"));
    expect(await screen.findByText("This work resumed safely.")).toBeVisible();
    expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("re-verifies a completed checkpoint without presenting a replay action", async () => {
    const onRetry = vi.fn(async () => undefined);
    const verificationReceipt = receipt({
      code: "mlc_verification_failed",
      boundary: "MlcVerifier",
      recoverable: true,
      recoveryAction: "resume_same_execution",
      message: "The final receipt could not be verified.",
      changedState: "checkpoint_saved",
      context: {},
    });
    const { container } = render(
      <RecoveryReceiptCard content={verificationReceipt} onRetry={onRetry} />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "One last check" })).toBeVisible();
    expect(screen.getByText(/without creating anything again/i)).toBeVisible();
    expect(screen.getByText(/No action will be replayed/i)).toBeVisible();
    expect(container.querySelector('[data-oomu-verification-recovery="verify_existing"]'))
      .toBeVisible();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Verify existing work" }));
    await waitFor(() => expect(onRetry).toHaveBeenCalledWith("agent-exec-test-6"));
  });

  it("restores the exact pending Mail approval after restart with a fresh token", async () => {
    const onRetry = vi.fn(async () => undefined);
    const frozenArgumentSha256 = "a".repeat(64);
    const interrupted = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      message: "OOMU restarted before the next confirmation.",
      context: {
        nextOperation: "draft_release_recovery_email",
        frozenArgumentSha256,
      },
    });

    expect(parseAgentExecutionRecoveryReceipt(interrupted)?.context).toMatchObject({
      nextOperation: "draft_release_recovery_email",
      frozenArgumentSha256,
    });
    const { container } = render(
      <RecoveryReceiptCard
        content={interrupted}
        executionState={{
          executionId: "agent-exec-test-6",
          planId: "plan-test-6",
          status: "halted",
          terminalPhase: "restart_recovery_ready",
          terminalVerified: false,
          verifiedComplete: false,
        }}
        executionStateStatus="ready"
        recoveryReceiptAuthority="current"
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "Mail approval ready to restore" })).toBeVisible();
    expect(screen.getByText(/exact unsent Mail draft is still pending; nothing was sent/i))
      .toBeVisible();
    expect(screen.getByText(/Completed work is saved.*new one-time approval token/i)).toBeVisible();
    expect(screen.queryByText(frozenArgumentSha256)).toBeNull();
    expect(container.querySelector('[data-oomu-interrupted-approval="mail_draft"]')).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Restore approval" }));
    await waitFor(() => expect(onRetry).toHaveBeenCalledWith("agent-exec-test-6"));
  });

  it("suppresses a restored Mail approval after its real resume action completes", () => {
    const onRetry = vi.fn(async () => undefined);
    const interrupted = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      context: {
        nextOperation: "draft_release_recovery_email",
        frozenArgumentSha256: "a".repeat(64),
      },
    });

    render(
      <RecoveryReceiptCard
        completedActionKeys={new Set(["agent-exec-test-6:resume_same_execution"])}
        content={interrupted}
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.queryByRole("button", { name: "Restore approval" })).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it("renders a durably completed Mail recovery as inert after relaunch", () => {
    const interrupted = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      context: {
        nextOperation: "draft_release_recovery_email",
        frozenArgumentSha256: "a".repeat(64),
      },
    });

    const { container } = render(
      <RecoveryReceiptCard
        content={interrupted}
        executionState={{
          executionId: "agent-exec-test-6",
          planId: "plan-test-6",
          status: "completed",
          terminalPhase: "completed",
          terminalVerified: true,
          verifiedComplete: true,
        }}
        executionStateStatus="ready"
        recoveryReceiptAuthority="inactive"
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    const card = screen.getByRole("region", { name: "Mail draft completed" });
    expect(card).toBeVisible();
    expect(card).toHaveClass("border-[var(--success)]/30", "bg-[var(--success-background)]");
    expect(container.querySelector('path[d="m5 12 4 4L19 6"]')).toBeVisible();
    expect(screen.getByText(/Nothing was sent, and this action will not replay/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Restore approval" })).toBeNull();
  });

  it("does not restore Mail approval from an unrelated halted execution", () => {
    const onRetry = vi.fn(async () => undefined);
    const interrupted = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      context: {
        nextOperation: "draft_release_recovery_email",
        frozenArgumentSha256: "a".repeat(64),
      },
    });

    render(
      <RecoveryReceiptCard
        content={interrupted}
        executionState={{
          executionId: "agent-exec-unrelated",
          planId: "plan-unrelated",
          status: "halted",
          terminalPhase: "restart_recovery_ready",
          terminalVerified: false,
          verifiedComplete: false,
        }}
        executionStateStatus="ready"
        recoveryReceiptAuthority="current"
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "Mail approval is no longer active" }))
      .toBeVisible();
    expect(screen.queryByRole("button", { name: "Restore approval" })).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it("requires the exact restart recovery phase before restoring Mail approval", () => {
    const onRetry = vi.fn(async () => undefined);
    const interrupted = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      context: {
        nextOperation: "draft_release_recovery_email",
        frozenArgumentSha256: "a".repeat(64),
      },
    });

    render(
      <RecoveryReceiptCard
        content={interrupted}
        executionState={{
          executionId: "agent-exec-test-6",
          planId: "plan-test-6",
          status: "halted",
          terminalPhase: "approval_pending",
          terminalVerified: false,
          verifiedComplete: false,
        }}
        executionStateStatus="ready"
        recoveryReceiptAuthority="current"
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.queryByRole("button", { name: "Restore approval" })).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it("keeps an inactive historical receipt clear and inert", () => {
    const onRetry = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_target_resolved",
          boundary: "CalendarRecovery",
          recoveryAction: "resume_same_execution",
          changedState: "checkpoint_saved",
          context: { requestedCalendarName: "OOMU Test" },
        })}
        recoveryReceiptAuthority="inactive"
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "This saved step is no longer active" }))
      .toBeVisible();
    expect(screen.getByText("No action will be replayed.")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it("shows no recovery action while receipt authority is being checked", () => {
    const onRetry = vi.fn(async () => undefined);
    const onRefreshExecutionState = vi.fn();
    render(
      <RecoveryReceiptCard
        content={receipt()}
        recoveryReceiptAuthority="checking"
        onRefreshExecutionState={onRefreshExecutionState}
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "Checking this saved step" })).toBeVisible();
    expect(screen.getByText(/No action is available while this check is in progress/i))
      .toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
    expect(onRefreshExecutionState).not.toHaveBeenCalled();
  });

  it("offers only a state refresh when receipt authority is unavailable", () => {
    const onRetry = vi.fn(async () => undefined);
    const onStartNewPlan = vi.fn(async () => undefined);
    const onRefreshExecutionState = vi.fn();
    render(
      <RecoveryReceiptCard
        content={receipt()}
        recoveryReceiptAuthority="unavailable"
        onRefreshExecutionState={onRefreshExecutionState}
        onRetry={onRetry}
        onStartNewPlan={onStartNewPlan}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "Couldn’t confirm this saved step" }))
      .toBeVisible();
    expect(screen.getAllByRole("button")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(onRefreshExecutionState).toHaveBeenCalledTimes(1);
    expect(onRetry).not.toHaveBeenCalled();
    expect(onStartNewPlan).not.toHaveBeenCalled();
  });

  it("does not elevate malformed restart context into a Mail approval restoration", () => {
    const malformed = receipt({
      code: "agent_execution_interrupted",
      boundary: "AgentExecutionRecovery",
      recoveryAction: "resume_same_execution",
      changedState: "checkpoint_saved",
      context: {
        nextOperation: "draft-release-recovery-email;send",
        frozenArgumentSha256: "not-a-digest",
      },
    });
    expect(parseAgentExecutionRecoveryReceipt(malformed)?.context).toMatchObject({
      nextOperation: null,
      frozenArgumentSha256: null,
    });
    const { container } = render(
      <RecoveryReceiptCard content={malformed} onRetry={vi.fn(async () => undefined)} />,
      { wrapper: I18nProvider },
    );
    expect(container.querySelector('[data-oomu-interrupted-approval="mail_draft"]')).toBeNull();
    expect(screen.queryByRole("button", { name: "Restore approval" })).toBeNull();
  });

});

describe("RecoveryReceiptCard calendar recovery", () => {
  it("never presents a native halt as completed", () => {
    render(<RecoveryReceiptCard content={receipt()} onRetry={vi.fn()} />, {
      wrapper: I18nProvider,
    });

    expect(screen.getByText("freight research needs another pass")).toBeVisible();
    expect(screen.queryByText(/completed|complete|ready/i)).toBeNull();
  });

  it("states that a denied Calendar action created nothing and preserves the exact next step", () => {
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_action_denied",
          boundary: "ShieldApprovalManager",
          recoveryAction: "resolve_calendar_target",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test Denial",
            availableCalendarNames: ["OOMU Test", "Personal"],
          },
        })}
        onResolveCalendar={vi.fn(async () => "resumed" as const)}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("region", { name: "Calendar event not approved" })).toBeVisible();
    expect(screen.getAllByText(/no event was created in “OOMU Test Denial”/i)).toHaveLength(2);
    expect(screen.getAllByText(/original agenda, frozen time slot, and verified checkpoint are saved/i))
      .toHaveLength(2);
    expect(screen.getAllByText(/Choose an existing calendar to continue from this exact step/i))
      .toHaveLength(2);
    expect(screen.getByText(/No Calendar change.*continue from the saved step/i)).toBeVisible();
    expect(screen.queryByText(/wasn’t found/i)).toBeNull();
    expect(screen.getByLabelText("Existing calendar")).toBeVisible();
    expect(screen.queryByRole("button", { name: /try again|retry/i })).toBeNull();
  });

  it("requires a calendar choice and never offers same-input retry", async () => {
    const onResolveCalendar = vi.fn(async (_executionId, choice) =>
      choice.resolution === "cancel" ? "cancelled" as const : "resumed" as const
    );
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_not_found",
          boundary: "Calendar",
          recoveryAction: "resolve_calendar_target",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test",
            availableCalendarNames: ["Work", "Personal", "Work"],
          },
        })}
        onResolveCalendar={onResolveCalendar}
      />,
      { wrapper: I18nProvider },
    );

    const card = screen.getByRole("region", { name: "Choose a calendar" });
    expect(card).toHaveAttribute("data-oomu-calendar-recovery-code", "calendar_not_found");
    expect(card).toHaveAttribute("data-oomu-calendar-recovery-action", "resolve_calendar_target");
    expect(card).toHaveAttribute("data-oomu-recovery-execution-id", "agent-exec-test-6");
    expect(screen.getByRole("button", { name: "Create “OOMU Test”" }))
      .toHaveAttribute("data-oomu-calendar-recovery", "create-requested");
    expect(screen.getByRole("button", { name: "Create “OOMU Test”" }))
      .toHaveAttribute("data-oomu-calendar-name", "OOMU Test");
    expect(screen.queryByRole("button", { name: /try again|retry/i })).toBeNull();
    fireEvent.change(screen.getByLabelText("Existing calendar"), { target: { value: "Personal" } });
    fireEvent.click(screen.getByRole("button", { name: "Use this calendar" }));
    await waitFor(() => expect(onResolveCalendar).toHaveBeenCalledWith(
      "agent-exec-test-6",
      { resolution: "select_existing", calendarName: "Personal" },
    ));
    expect(await screen.findByText(/continuing from the paused step/i)).toBeVisible();
  });

  it("renders a durable calendar resolution as completed without a duplicate retry", () => {
    render(
      <RecoveryReceiptCard
        completedActionKeys={new Set(["agent-exec-test-6:resume_same_execution"])}
        content={receipt({
          code: "calendar_target_resolved",
          boundary: "CalendarRecovery",
          recoveryAction: "resume_same_execution",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test",
            selectedCalendarName: "Personal",
            resolution: "selected_existing",
          },
        })}
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    const card = screen.getByRole("region", { name: "Calendar ready" });
    expect(card).toHaveAttribute(
      "data-oomu-calendar-recovery-action",
      "calendar_target_resolved",
    );
    expect(screen.getByText(/calendar is set/i)).toBeVisible();
    expect(screen.getByText(/resumed safely/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: /try again|retry/i })).toBeNull();
  });

  it("can continue a durably resolved calendar if the worker did not start", async () => {
    const onRetry = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_target_resolved",
          boundary: "CalendarRecovery",
          recoveryAction: "resume_same_execution",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test",
            selectedCalendarName: "Personal",
            resolution: "selected_existing",
          },
        })}
        onRetry={onRetry}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Continue from saved step" }));
    await waitFor(() => expect(onRetry).toHaveBeenCalledWith("agent-exec-test-6"));
    expect(screen.queryByRole("button", { name: "Continue from saved step" })).toBeNull();
  });

  it("explains a changed calendar choice and keeps recovery actionable", async () => {
    const failure = Object.assign(new Error("Calendar is incompatible"), {
      code: "calendar_availability_unsupported",
    });
    const onResolveCalendar = vi.fn(async () => {
      throw failure;
    });
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_not_found",
          boundary: "Calendar",
          recoveryAction: "resolve_calendar_target",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test",
            availableCalendarNames: ["Family", "Personal"],
          },
        })}
        onResolveCalendar={onResolveCalendar}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(screen.getByLabelText("Existing calendar"), {
      target: { value: "Family" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Use this calendar" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The calendar “Family” can’t mark events as tentative",
    );
    expect(screen.getByRole("button", { name: "Use this calendar" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Create “OOMU Test”" })).toBeEnabled();
  });

  it("offers Calendar Settings if access changes during target resolution", async () => {
    const onOpenCalendarSettings = vi.fn(async () => undefined);
    const onResolveCalendar = vi.fn(async () => {
      throw Object.assign(new Error("Calendar access changed"), {
        code: "calendar_permission_denied",
      });
    });
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_not_found",
          boundary: "Calendar",
          recoveryAction: "resolve_calendar_target",
          changedState: "checkpoint_saved",
          context: {
            requestedCalendarName: "OOMU Test",
            availableCalendarNames: ["Personal"],
          },
        })}
        onOpenCalendarSettings={onOpenCalendarSettings}
        onResolveCalendar={onResolveCalendar}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.change(screen.getByLabelText("Existing calendar"), {
      target: { value: "Personal" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Use this calendar" }));
    expect(await screen.findByRole("button", { name: "Open Calendar Settings" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Open Calendar Settings" }));
    await waitFor(() => expect(onOpenCalendarSettings).toHaveBeenCalledWith(
      "agent-exec-test-6",
    ));
    expect(screen.getByRole("button", { name: "Use this calendar" })).toBeEnabled();
  });

  it.each([
    ["calendar_permission_denied", /needs Full Access to Calendar to check conflicts/i],
    ["calendar_permission_restricted", /needs Full Access to Calendar to check conflicts/i],
    ["calendar_permission_write_only", /can add an event, but it needs Full Access/i],
    ["calendar_permission_unavailable", /isn’t available right now/i],
    ["calendar_authorization_timeout", /took too long to confirm access/i],
  ])("renders a dedicated Full Access recovery for %s", (code, body) => {
    expect(isCalendarPermissionRecoveryCode(code)).toBe(true);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Calendar",
          changedState: "checkpoint_saved",
          code,
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={vi.fn(async () => undefined)}
        onOpenCalendarSettings={vi.fn(async () => undefined)}
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    const card = screen.getByRole("region", { name: "Calendar needs Full Access" });
    expect(card).toHaveAttribute("data-oomu-calendar-recovery-action", "restore_calendar_full_access");
    expect(card).toHaveAttribute("data-oomu-calendar-recovery-code", code);
    expect(card).toHaveAttribute("data-oomu-recovery-execution-id", "agent-exec-test-6");
    expect(screen.getAllByText(body)[0]).toBeVisible();
    expect(screen.getAllByText(/completed work is saved/i)[0]).toBeVisible();
    expect(screen.getByRole("button", { name: "Open Calendar Settings" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Check access and continue" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel remaining work" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /create/i })).toBeNull();
  });

  it("opens Calendar Settings without consuming the recovery, then resumes the same execution", async () => {
    const onOpenCalendarSettings = vi.fn(async () => undefined);
    const onCheckCalendarAccess = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Calendar",
          changedState: "checkpoint_saved",
          code: "calendar_permission_denied",
          recoveryAction: "resume_same_execution",
        })}
        onCheckCalendarAccess={onCheckCalendarAccess}
        onOpenCalendarSettings={onOpenCalendarSettings}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Open Calendar Settings" }));
    await waitFor(() => expect(onOpenCalendarSettings).toHaveBeenCalledWith("agent-exec-test-6"));
    expect(screen.getByRole("button", { name: "Check access and continue" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Check access and continue" }));
    await waitFor(() => expect(onCheckCalendarAccess).toHaveBeenCalledWith("agent-exec-test-6"));
    expect(await screen.findByText(/Calendar access is ready/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Check access and continue" })).toBeNull();
  });

  it("keeps a failed Calendar access check actionable and explains the failure", async () => {
    const onCheckCalendarAccess = vi.fn(async () => {
      throw new Error("Calendar permission remains unavailable");
    });
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Calendar",
          changedState: "checkpoint_saved",
          code: "calendar_permission_denied",
          recoveryAction: "resume_same_execution",
        })}
        onCheckCalendarAccess={onCheckCalendarAccess}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Check access and continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU couldn’t confirm Calendar access",
    );
    expect(screen.getByRole("button", { name: "Check access and continue" })).toBeEnabled();
  });

  it("cancels only the remaining work and reports preservation of completed work", async () => {
    const onCancelRemainingWork = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Calendar",
          changedState: "checkpoint_saved",
          code: "calendar_permission_write_only",
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={onCancelRemainingWork}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Cancel remaining work" }));
    await waitFor(() => expect(onCancelRemainingWork).toHaveBeenCalledWith("agent-exec-test-6"));
    expect(await screen.findByText(/remaining work was cancelled/i)).toBeVisible();
    expect(screen.getByText(/completed work is preserved/i)).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("keeps a durably cancelled permission recovery closed after remount", () => {
    render(
      <RecoveryReceiptCard
        completedActionKeys={new Set(["agent-exec-test-6:cancel_remaining_work"])}
        content={receipt({
          boundary: "Calendar",
          changedState: "checkpoint_saved",
          code: "calendar_permission_denied",
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText(/remaining work was cancelled/i)).toBeVisible();
    expect(screen.getByText(/completed work is preserved/i)).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

});

describe("RecoveryReceiptCard durable outcomes", () => {
  it("renders the backend cancellation receipt as a durable completed outcome", () => {
    const durableCancellation = receipt({
      boundary: "AgentExecutionRecovery",
      changedState: "checkpoint_saved",
      code: "agent_execution_remaining_work_cancelled",
      context: { cancelled: true, completedStepCount: 2 },
      message: "Remaining work stopped. Completed work remains preserved.",
      recoverable: false,
      recoveryAction: "remaining_work_cancelled",
    });

    expect(parseAgentExecutionRecoveryReceipt(durableCancellation)).toMatchObject({
      code: "agent_execution_remaining_work_cancelled",
      recoverable: false,
      recoveryAction: "remaining_work_cancelled",
    });

    render(<RecoveryReceiptCard content={durableCancellation} />, {
      wrapper: I18nProvider,
    });

    expect(screen.getByRole("region", { name: "Remaining work cancelled" })).toBeVisible();
    expect(screen.getAllByText(/remaining work was cancelled/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/completed work is preserved/i).length).toBeGreaterThan(0);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it.each([
    [
      "mail_automation_permission_required",
      "Mail needs Automation access",
      /hasn’t allowed OOMU to control Mail yet/i,
      true,
    ],
    [
      "mail_automation_timeout",
      "Mail didn’t respond in time",
      /didn’t confirm Automation access in time/i,
      false,
    ],
    [
      "mail_automation_unavailable",
      "Mail is unavailable right now",
      /can’t reach Mail Automation right now/i,
      false,
    ],
  ])("renders honest Mail Automation recovery for %s", (code, title, body, showsSettings) => {
    expect(isMailAutomationRecoveryCode(code)).toBe(true);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Mail",
          changedState: "checkpoint_saved",
          code,
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={vi.fn(async () => undefined)}
        onCheckMailAutomationAccess={vi.fn(async () => undefined)}
        onOpenMailAutomationSettings={vi.fn(async () => undefined)}
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    const card = screen.getByRole("region", { name: title });
    expect(card).toHaveAttribute("data-oomu-mail-recovery-code", code);
    expect(screen.getAllByText(body)[0]).toBeVisible();
    expect(screen.getAllByText(/no Mail draft was created/i)[0]).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel remaining work" })).toBeVisible();
    if (showsSettings) {
      expect(card).toHaveAttribute(
        "data-oomu-mail-recovery-action",
        "restore_mail_automation_access",
      );
      expect(screen.getByRole("button", { name: "Open Automation Settings" })).toBeVisible();
      expect(screen.getByRole("button", { name: "Check access and continue" })).toBeVisible();
      expect(screen.queryByRole("button", { name: "Retry and continue" })).toBeNull();
    } else {
      expect(card).toHaveAttribute("data-oomu-mail-recovery-action", "retry_mail_automation");
      expect(screen.getByRole("button", { name: "Retry and continue" })).toBeVisible();
      expect(screen.queryByRole("button", { name: "Open Automation Settings" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Check access and continue" })).toBeNull();
    }
  });

  it.each([
    "mail_automation_permission_required",
    "mail_automation_timeout",
    "mail_automation_unavailable",
  ])("%s describes a paused send truthfully", (code) => {
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Mail",
          changedState: "checkpoint_saved",
          code,
          context: { nextOperation: "send_system_email" },
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={vi.fn(async () => undefined)}
        onCheckMailAutomationAccess={vi.fn(async () => undefined)}
        onOpenMailAutomationSettings={vi.fn(async () => undefined)}
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getAllByText(/no email was sent/i)[0]).toBeVisible();
    expect(screen.queryByText(/no Mail draft was created/i)).toBeNull();
  });

  it("opens Automation Settings without consuming recovery, then checks and resumes", async () => {
    const onOpenMailAutomationSettings = vi.fn(async () => undefined);
    const onCheckMailAutomationAccess = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Mail",
          changedState: "checkpoint_saved",
          code: "mail_automation_permission_required",
          recoveryAction: "resume_same_execution",
        })}
        onCheckMailAutomationAccess={onCheckMailAutomationAccess}
        onOpenMailAutomationSettings={onOpenMailAutomationSettings}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Open Automation Settings" }));
    await waitFor(() => expect(onOpenMailAutomationSettings).toHaveBeenCalledWith(
      "agent-exec-test-6",
    ));
    expect(screen.getByRole("button", { name: "Check access and continue" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Check access and continue" }));
    await waitFor(() => expect(onCheckMailAutomationAccess).toHaveBeenCalledWith(
      "agent-exec-test-6",
    ));
    expect(await screen.findByText(/Mail access is ready/i)).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it.each(["mail_automation_timeout", "mail_automation_unavailable"])(
    "%s retries the exact checkpoint without opening Settings",
    async (code) => {
      const onOpenMailAutomationSettings = vi.fn(async () => undefined);
      const onRetry = vi.fn(async () => undefined);
      render(
        <RecoveryReceiptCard
          content={receipt({
            boundary: "Mail",
            changedState: "checkpoint_saved",
            code,
            recoveryAction: "resume_same_execution",
          })}
          onCancelRemainingWork={vi.fn(async () => undefined)}
          onOpenMailAutomationSettings={onOpenMailAutomationSettings}
          onRetry={onRetry}
        />,
        { wrapper: I18nProvider },
      );

      expect(screen.queryByRole("button", { name: "Open Automation Settings" })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Retry and continue" }));
      await waitFor(() => expect(onRetry).toHaveBeenCalledWith("agent-exec-test-6"));
      expect(onOpenMailAutomationSettings).not.toHaveBeenCalled();
      expect(await screen.findByText(/Mail access is ready/i)).toBeVisible();
    },
  );

  it("durably cancels the remaining Mail work without losing completed work", async () => {
    const onCancelRemainingWork = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          boundary: "Mail",
          changedState: "checkpoint_saved",
          code: "mail_automation_unavailable",
          recoveryAction: "resume_same_execution",
        })}
        onCancelRemainingWork={onCancelRemainingWork}
        onRetry={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Cancel remaining work" }));
    await waitFor(() => expect(onCancelRemainingWork).toHaveBeenCalledWith(
      "agent-exec-test-6",
    ));
    expect(await screen.findByText(/remaining work was cancelled/i)).toBeVisible();
    expect(screen.getByText(/completed work is preserved/i)).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("renders cancellation as cancellation instead of continuation", async () => {
    const onResolveCalendar = vi.fn(async () => "cancelled" as const);
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_not_found",
          boundary: "Calendar",
          recoveryAction: "resolve_calendar_target",
          context: { requestedCalendarName: "OOMU Test", availableCalendarNames: [] },
        })}
        onResolveCalendar={onResolveCalendar}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel task" }));
    expect(await screen.findByText("The paused task was cancelled.")).toBeVisible();
    expect(screen.queryByText(/continuing from the paused step/i)).toBeNull();
  });

  it("keeps a durably cancelled calendar recovery closed after remount", () => {
    const onResolveCalendar = vi.fn();
    render(
      <RecoveryReceiptCard
        content={receipt({
          code: "calendar_recovery_cancelled",
          boundary: "CalendarRecovery",
          recoverable: false,
          recoveryAction: "calendar_recovery_cancelled",
          changedState: "checkpoint_saved",
          context: { requestedCalendarName: "OOMU Test", cancelled: true },
        })}
        onResolveCalendar={onResolveCalendar}
      />,
      { wrapper: I18nProvider },
    );
    expect(screen.getAllByText("The paused task was cancelled.").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button")).toBeNull();
    expect(onResolveCalendar).not.toHaveBeenCalled();
  });

  it("does not offer Retry for a nonrecoverable receipt", () => {
    render(
      <RecoveryReceiptCard
        content={receipt({ recoverable: false, code: "agent_execution_failed" })}
        onRetry={vi.fn()}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.queryByRole("button", { name: /retry|try again/i })).toBeNull();
  });

  it("offers a fresh plan for the legacy Test 7 zero-change receipt", async () => {
    const onStartNewPlan = vi.fn(async () => undefined);
    const legacyReceipt = receipt({
      recoverable: false,
      code: "preflight_verification_failed",
      recoveryAction: undefined,
    });
    expect(parseAgentExecutionRecoveryReceipt(legacyReceipt)?.recoveryAction).toBe(
      "start_new_plan",
    );
    render(
      <RecoveryReceiptCard
        content={legacyReceipt}
        onRetry={vi.fn()}
        onStartNewPlan={onStartNewPlan}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Start a new plan" }));
    await waitFor(() => {
      expect(onStartNewPlan).toHaveBeenCalledWith("agent-exec-test-6");
    });
    expect(await screen.findByText(/new plan is ready/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Start a new plan" })).toBeNull();
    expect(screen.queryByRole("button", { name: /retry|try again/i })).toBeNull();
  });

  it("keeps a failed fresh-plan action visibly retryable", async () => {
    const onStartNewPlan = vi.fn()
      .mockRejectedValueOnce(new Error("planner unavailable"))
      .mockResolvedValueOnce(undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          recoverable: false,
          recoveryAction: "start_new_plan",
          code: "preflight_verification_failed",
        })}
        onStartNewPlan={onStartNewPlan}
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("button", { name: "Start a new plan" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/couldn’t start a new plan/i);
    fireEvent.click(screen.getByRole("button", { name: "Start a new plan" }));
    await waitFor(() => expect(onStartNewPlan).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(/new plan is ready/i)).toBeVisible();
  });

  it("keeps a completed action single-use when its receipt remounts", () => {
    render(
      <RecoveryReceiptCard
        completedActionKeys={new Set(["agent-exec-test-6:start_new_plan"])}
        content={receipt({
          recoverable: false,
          recoveryAction: "start_new_plan",
          code: "preflight_verification_failed",
        })}
        onStartNewPlan={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText(/new plan is ready/i)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Start a new plan" })).toBeNull();
  });

  it("honors the explicit new-plan action without resuming the stopped execution", () => {
    const onRetry = vi.fn(async () => undefined);
    const startReceipt = receipt({
      recoverable: false,
      recoveryAction: "start_new_plan",
      code: "preflight_verification_failed",
    });
    render(
      <RecoveryReceiptCard
        content={startReceipt}
        onRetry={onRetry}
        onStartNewPlan={vi.fn(async () => undefined)}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("button", { name: "Start a new plan" })).toBeVisible();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it.each(["checkpoint_saved", "external_changes"])(
    "maps a legacy nonrecoverable %s receipt to review-only guidance",
    (changedState) => {
      expect(parseAgentExecutionRecoveryReceipt(receipt({
        recoverable: false,
        recoveryAction: undefined,
        changedState,
      }))?.recoveryAction).toBe("review_external_changes");
    },
  );

  it("shows review guidance for external changes and never offers replay", () => {
    const onRetry = vi.fn(async () => undefined);
    const onStartNewPlan = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={receipt({
          recoverable: false,
          recoveryAction: "review_external_changes",
          changedState: "external_changes",
        })}
        onRetry={onRetry}
        onStartNewPlan={onStartNewPlan}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText(/won’t replay this execution/i)).toBeVisible();
    expect(screen.getByText(/tell OOMU in chat what to keep or redo/i)).toBeVisible();
    expect(screen.getByText("Decision pack research")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
    expect(onRetry).not.toHaveBeenCalled();
    expect(onStartNewPlan).not.toHaveBeenCalled();
  });
});
