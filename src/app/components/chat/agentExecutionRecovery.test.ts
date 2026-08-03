import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelRemainingAgentWorkForSession,
  checkCalendarFullAccessAndResumeForSession,
  checkMailAutomationAccessAndResumeForSession,
  getAgentExecutionRecoveryStates,
  localizedAgentPlanSummary,
  openCalendarPrivacySettings,
  openMailAutomationSettings,
  prepareAgentExecutionReplan,
  resolveAgentCalendarRecoveryForSession,
  resumeAgentExecutionForSession,
  startNewAgentRecoveryPlan,
} from "./agentExecutionRecovery";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauriRuntime: true,
}));

function recoveryOwnership(executionId = "execution-7") {
  const suffix = executionId.slice("execution-".length);
  return {
    sessionId: "session-7",
    rootTurnId: `turn-root-${suffix}`,
    failedTurnId: `turn-failed-${suffix}`,
    generationToken: `generation-${suffix}`,
  };
}

describe("agent execution recovery", () => {
  beforeEach(() => invokeMock.mockReset());

  it("prepares only the exact execution in the active session and preserves its objective", async () => {
    invokeMock.mockResolvedValue({ objective: "  Rebuild the supplier decision pack safely.  " });

    await expect(
      prepareAgentExecutionReplan(" execution-7 ", " session-7 "),
    ).resolves.toBe("  Rebuild the supplier decision pack safely.  ");
    expect(invokeMock).toHaveBeenCalledWith("prepare_agent_execution_replan", {
      request: { executionId: "execution-7", sessionId: "session-7" },
    });
  });

  it("rejects an empty durable objective instead of submitting invented work", async () => {
    invokeMock.mockResolvedValue({ objective: "   " });

    await expect(
      prepareAgentExecutionReplan("execution-7", "session-7"),
    ).rejects.toThrow("agent_execution_replan_objective_missing");
  });

  it("does not report recovery success until an approval-ready plan exists", async () => {
    invokeMock.mockResolvedValue({ objective: "Prepare the verified decision pack." });
    const submit = vi.fn(async (_objective, options) => options.onAccepted());

    await expect(startNewAgentRecoveryPlan({
      executionId: "execution-7",
      sessionId: "session-7",
      currentSessionId: () => "session-7",
      submit,
    })).rejects.toThrow("agent_execution_replan_plan_missing");
  });

  it("resolves only after the isolated recovery submission produces a plan", async () => {
    invokeMock.mockResolvedValue({ objective: "Prepare the verified decision pack." });
    const submit = vi.fn(async (_objective, options) => {
      options.onAccepted();
      options.onPlanReady();
    });

    await expect(startNewAgentRecoveryPlan({
      executionId: "execution-7",
      sessionId: "session-7",
      currentSessionId: () => "session-7",
      submit,
    })).resolves.toBeUndefined();
    expect(submit).toHaveBeenCalledWith(
      "Prepare the verified decision pack.",
      expect.objectContaining({ expectedSessionId: "session-7", recoveryPlan: true }),
    );
  });

  it("drops recovery before submission when the user switches sessions", async () => {
    invokeMock.mockResolvedValue({ objective: "Prepare the verified decision pack." });
    const submit = vi.fn();

    await expect(startNewAgentRecoveryPlan({
      executionId: "execution-7",
      sessionId: "session-7",
      currentSessionId: () => "session-8",
      submit,
    })).rejects.toThrow("agent_execution_replan_ownership_mismatch");
    expect(submit).not.toHaveBeenCalled();
  });

  it("localizes every user-visible line in the durable plan summary", () => {
    const translate = vi.fn((key: string, variables?: Record<string, string | number>) =>
      `${key}:${JSON.stringify(variables ?? {})}`
    );
    const summary = localizedAgentPlanSummary(translate, {
      id: "plan-7",
      objective: "Prepare the decision pack.",
      steps: [{}, {}],
    });

    expect(summary).toContain("chat.recovery.plan_compiled");
    expect(summary).toContain('chat.recovery.plan_steps:{"count":2}');
    expect(translate).toHaveBeenCalledTimes(4);
  });

  it("loads only bounded recovery states owned by the active session", async () => {
    invokeMock.mockResolvedValue([{
      executionId: "execution-7",
      planId: "plan-7", ...recoveryOwnership(),
      status: "completed",
      terminalPhase: "completed",
      terminalVerified: true,
      verifiedComplete: true,
    }]);

    await expect(getAgentExecutionRecoveryStates(
      "session-7",
      ["execution-7", "execution-7"],
    )).resolves.toEqual([expect.objectContaining({
      executionId: "execution-7",
      planId: "plan-7",
      status: "completed",
      verifiedComplete: true,
    })]);
    expect(invokeMock).toHaveBeenCalledWith("get_agent_execution_recovery_states", {
      sessionId: "session-7",
      executionIds: ["execution-7"],
    });
  });

  it("queries 65 recovery states in bounded batches and merges their projections", async () => {
    const executionIds = Array.from({ length: 65 }, (_, index) => `execution-${index + 1}`);
    const statesFor = (ids: string[]) => ids.map((executionId) => ({
      executionId,
      planId: `plan-${executionId.slice("execution-".length)}`, ...recoveryOwnership(executionId),
      status: "completed",
      terminalPhase: "completed",
      terminalVerified: true,
      verifiedComplete: true,
    }));
    invokeMock
      .mockResolvedValueOnce(statesFor(executionIds.slice(0, 64)))
      .mockResolvedValueOnce(statesFor(executionIds.slice(64)));

    const states = await getAgentExecutionRecoveryStates("session-7", executionIds);

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_agent_execution_recovery_states", {
      sessionId: "session-7",
      executionIds: executionIds.slice(0, 64),
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "get_agent_execution_recovery_states", {
      sessionId: "session-7",
      executionIds: executionIds.slice(64),
    });
    expect(states).toHaveLength(65);
    expect(states.map((state) => state.executionId)).toEqual(executionIds);
  });

  it("validates every requested execution before starting any recovery-state batch", async () => {
    const executionIds = Array.from({ length: 65 }, (_, index) => `execution-${index + 1}`);
    executionIds[64] = "invalid execution";

    await expect(getAgentExecutionRecoveryStates("session-7", executionIds))
      .rejects.toThrow("agent_execution_recovery_states_unavailable");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("normalizes an omitted legacy terminal phase to null", async () => {
    invokeMock.mockResolvedValue([{
      executionId: "execution-7",
      planId: "plan-7", ...recoveryOwnership(),
      status: "running",
      terminalVerified: false,
      verifiedComplete: false,
    }]);

    await expect(getAgentExecutionRecoveryStates("session-7", ["execution-7"]))
      .resolves.toEqual([{
        executionId: "execution-7",
        planId: "plan-7", ...recoveryOwnership(),
        status: "running",
        terminalPhase: null,
        terminalVerified: false,
        verifiedComplete: false,
      }]);
  });

  it("rejects foreign, duplicated, or malformed recovery-state projections", async () => {
    invokeMock.mockResolvedValue([{
      executionId: "another-execution",
      planId: "plan-7", ...recoveryOwnership("another-execution"),
      status: "completed",
      terminalPhase: "completed",
      terminalVerified: true,
      verifiedComplete: true,
    }]);
    await expect(getAgentExecutionRecoveryStates("session-7", ["execution-7"]))
      .rejects.toThrow("agent_execution_recovery_states_invalid");
  });

  it("rejects a resumed execution owned by another session", async () => {
    invokeMock.mockResolvedValue({
      executionId: "execution-7",
      planId: "plan-7",
      sessionId: "other-session",
      streamStartAfterLogId: 12,
    });

    await expect(
      resumeAgentExecutionForSession("execution-7", "session-7"),
    ).rejects.toThrow("agent_execution_resume_ownership_mismatch");
  });

  it("resolves the target before resuming the exact execution", async () => {
    invokeMock
      .mockResolvedValueOnce({ status: "ready_to_resume", selectedCalendarName: "Personal" })
      .mockResolvedValueOnce({
        executionId: "execution-7",
        planId: "plan-7",
        sessionId: "session-7",
        streamStartAfterLogId: 18,
      });

    const result = await resolveAgentCalendarRecoveryForSession(
      "execution-7",
      "session-7",
      { resolution: "select_existing", calendarName: "Personal" },
    );
    expect(result.status).toBe("resumed");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "resolve_agent_calendar_recovery", {
      request: {
        executionId: "execution-7",
        sessionId: "session-7",
        resolution: "select_existing",
        calendarName: "Personal",
      },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "resume_agent_execution", {
      request: { executionId: "execution-7" },
    });
  });

  it("cancels without calling resume", async () => {
    invokeMock.mockResolvedValue({ status: "cancelled" });
    await expect(resolveAgentCalendarRecoveryForSession(
      "execution-7",
      "session-7",
      { resolution: "cancel" },
    )).resolves.toEqual({ status: "cancelled", execution: null });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("opens the exact native Calendar privacy settings surface", async () => {
    invokeMock.mockResolvedValue(undefined);
    await expect(openCalendarPrivacySettings()).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("open_calendar_privacy_settings");
  });

  it("checks Full Access before resuming the exact stopped execution", async () => {
    invokeMock
      .mockResolvedValueOnce({ status: "full_access", fullAccess: true, canRequestFullAccess: false })
      .mockResolvedValueOnce({
        executionId: "execution-7",
        planId: "plan-7",
        sessionId: "session-7",
        streamStartAfterLogId: 21,
      });

    const execution = await checkCalendarFullAccessAndResumeForSession(
      "execution-7",
      "session-7",
    );
    expect(execution.executionId).toBe("execution-7");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "check_calendar_full_access");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "resume_agent_execution", {
      request: { executionId: "execution-7" },
    });
  });

  it.each([
    ["not_determined", true],
    ["write_only", true],
    ["denied", false],
    ["restricted", false],
    ["unavailable", false],
  ])("does not resume when the native Calendar recovery returns %s", async (
    status,
    canRequestFullAccess,
  ) => {
    invokeMock.mockResolvedValue({
      status,
      fullAccess: false,
      canRequestFullAccess,
    });
    await expect(checkCalendarFullAccessAndResumeForSession(
      "execution-7",
      "session-7",
    )).rejects.toThrow("calendar_full_access_required");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("check_calendar_full_access");
  });

  it("opens the exact native Mail Automation settings surface", async () => {
    invokeMock.mockResolvedValue(undefined);
    await expect(openMailAutomationSettings()).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("open_mail_automation_settings");
  });

  it.each([
    { status: "authorized", authorized: true, retrySupported: true },
    { status: "target_not_running", authorized: false, retrySupported: true },
  ])("checks Mail Automation before resuming the exact stopped execution: $status", async (access) => {
    invokeMock
      .mockResolvedValueOnce(access)
      .mockResolvedValueOnce({
        executionId: "execution-7",
        planId: "plan-7",
        sessionId: "session-7",
        streamStartAfterLogId: 22,
      });

    const execution = await checkMailAutomationAccessAndResumeForSession(
      "execution-7",
      "session-7",
    );
    expect(execution.executionId).toBe("execution-7");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "check_mail_automation_access");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "resume_agent_execution", {
      request: { executionId: "execution-7" },
    });
  });

  it.each([
    { status: "permission_required", authorized: false, retrySupported: false },
    { status: "unavailable", authorized: false, retrySupported: false },
    { status: "timeout", authorized: false, retrySupported: false },
  ])("does not resume when Mail Automation remains $status", async (access) => {
    invokeMock.mockResolvedValue(access);
    await expect(checkMailAutomationAccessAndResumeForSession(
      "execution-7",
      "session-7",
    )).rejects.toThrow("mail_automation_permission_required");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "resume_agent_execution",
      expect.anything(),
    );
  });

  it("durably cancels only the remaining work for the owned execution", async () => {
    invokeMock.mockResolvedValue({ status: "cancelled" });
    await expect(cancelRemainingAgentWorkForSession(
      "execution-7",
      "session-7",
    )).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("cancel_agent_execution_remaining_work", {
      request: { executionId: "execution-7", sessionId: "session-7" },
    });
  });

  it("does not display cancellation when the durable transition is rejected", async () => {
    invokeMock.mockResolvedValue({ status: "running" });
    await expect(cancelRemainingAgentWorkForSession(
      "execution-7",
      "session-7",
    )).rejects.toThrow("agent_execution_cancel_invalid");
  });
});
