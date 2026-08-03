import { describe, expect, it, vi } from "vitest";
import { agentRecoveryActionKey } from "./agentExecutionRecovery";
import {
  calendarNameFromRecoveryFollowup,
  calendarRecoveryFollowupForTranscript,
  resolveCalendarRecoveryFollowup,
} from "./calendarRecoveryFollowup";

function receipt(
  executionId: string,
  recoveryAction: "resolve_calendar_target" | "resume_same_execution",
) {
  return JSON.stringify({
    schema: "oomu.agent_execution_recovery.v1",
    executionId,
    planId: `plan-${executionId}`,
    code: recoveryAction === "resolve_calendar_target"
      ? "calendar_not_found"
      : "calendar_target_resolved",
    boundary: "Calendar",
    recoverable: true,
    recoveryAction,
    message: "Calendar recovery state.",
    changedState: "checkpoint_saved",
    context: {
      requestedCalendarName: "OOMU Test Denial",
      availableCalendarNames: ["OOMU Test", "Family"],
    },
  });
}

describe("calendar recovery follow-up", () => {
  it.each([
    ["Use my OOMU Test calendar instead and continue.", "OOMU Test"],
    ["use my “Family” calendar instead and continue", "Family"],
  ])("extracts the bounded exact correction from %s", (message, expected) => {
    expect(calendarNameFromRecoveryFollowup(message)).toBe(expected);
  });

  it.each([
    "Use OOMU Test instead.",
    "Create my OOMU Test calendar and continue.",
    `Use my ${"x".repeat(81)} calendar instead and continue.`,
  ])("does not broaden unrelated calendar language: %s", (message) => {
    expect(calendarNameFromRecoveryFollowup(message)).toBeNull();
  });

  it("binds only to the newest unresolved durable receipt in this transcript", () => {
    const transcript = [
      { role: "assistant" as const, content: receipt("execution-older", "resolve_calendar_target") },
      { role: "user" as const, content: "Try something else." },
      { role: "assistant" as const, content: receipt("execution-current", "resolve_calendar_target") },
    ];

    expect(calendarRecoveryFollowupForTranscript(
      "Use my OOMU Test calendar instead and continue.",
      transcript,
    )).toEqual({ calendarName: "OOMU Test", executionId: "execution-current" });
  });

  it("does not revive a receipt that was resolved before relaunch", () => {
    expect(calendarRecoveryFollowupForTranscript(
      "Use my OOMU Test calendar instead and continue.",
      [{ role: "assistant", content: receipt("execution-7", "resume_same_execution") }],
    )).toBeNull();
  });

  it("does not reuse a receipt already consumed in the current session", () => {
    expect(calendarRecoveryFollowupForTranscript(
      "Use my OOMU Test calendar instead and continue.",
      [{ role: "assistant", content: receipt("execution-7", "resolve_calendar_target") }],
      new Set([agentRecoveryActionKey("execution-7", "resolve_calendar_target")]),
    )).toBeNull();
  });

  it("does not skip a newer non-calendar recovery to capture stale work", () => {
    const newerMailReceipt = JSON.stringify({
      schema: "oomu.agent_execution_recovery.v1",
      executionId: "execution-mail",
      planId: "plan-mail",
      code: "mail_automation_permission_required",
      boundary: "Mail",
      recoverable: true,
      recoveryAction: "resume_same_execution",
      message: "Mail access is required.",
      changedState: "checkpoint_saved",
      context: {},
    });
    expect(calendarRecoveryFollowupForTranscript(
      "Use my OOMU Test calendar instead and continue.",
      [
        { role: "assistant", content: receipt("execution-calendar", "resolve_calendar_target") },
        { role: "assistant", content: newerMailReceipt },
      ],
    )).toBeNull();
  });

  it("resolves the exact calendar correction and returns the success presentation", async () => {
    const resolve = vi.fn().mockResolvedValue("resumed");
    await expect(resolveCalendarRecoveryFollowup(
      { calendarName: "OOMU Test", executionId: "execution-7" },
      resolve,
    )).resolves.toEqual({
      contentKey: "chat.recovery.calendar_resolved",
      role: "assistant",
      status: "completed",
      statusKey: "chat.recovery.calendar_continuing",
    });
    expect(resolve).toHaveBeenCalledWith("execution-7", {
      resolution: "select_existing",
      calendarName: "OOMU Test",
    });
  });

  it("keeps an untyped resolver failure on the stopped recovery", async () => {
    await expect(resolveCalendarRecoveryFollowup(
      { calendarName: "OOMU Test", executionId: "execution-7" },
      async () => "cancelled",
    )).resolves.toEqual({
      contentKey: "chat.recovery.calendar_failed",
      role: "system",
      status: "failed",
      statusKey: "chat.status.halted",
    });
  });

  it.each([
    ["calendar_not_found", "chat.recovery.calendar_missing_body"],
    ["calendar_name_ambiguous", "chat.recovery.calendar_ambiguous_body"],
    ["calendar_read_only", "chat.recovery.calendar_read_only_body"],
    ["calendar_availability_unsupported", "chat.recovery.calendar_incompatible_body"],
  ] as const)("preserves the actionable native %s failure", async (code, contentKey) => {
    await expect(resolveCalendarRecoveryFollowup(
      { calendarName: "OOMU Test", executionId: "execution-7" },
      async () => { throw { code, message: "Native Calendar validation failed." }; },
    )).resolves.toEqual({
      contentKey,
      contentVariables: { calendar: "OOMU Test" },
      role: "system",
      status: "failed",
      statusKey: "chat.status.halted",
    });
  });
});
