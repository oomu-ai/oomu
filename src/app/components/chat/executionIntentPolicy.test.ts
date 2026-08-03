import { describe, expect, it } from "vitest";
import {
  bindNativeEffectExpectationToTool,
  containsUnverifiedActionClaim,
  evaluateLikelyLocalNativeTaskIntent,
  hasRecurringAutomationIntent,
  hasStructuralExecutionIntent,
  isRetrospectiveNativeActionQuestion,
  nativeEffectExpectationForRouteDecision,
  outstandingNativeEffectAfterReceipt,
  requiresPendingNativePostcondition,
  shouldBlockUnverifiedActionClaim,
} from "./executionIntentPolicy";

describe("native execution receipt policy", () => {
  it("treats recurring app work as structural execution", () => {
    const prompt = "Can you set up an hourly task to check my email for any unread messages. Only run for today until midnight tonight. Once you set it up, run it once to ensure it’s working properly. If it does not work properly, report back here and let me know the outcome.";

    expect(hasRecurringAutomationIntent(prompt)).toBe(true);
    expect(hasStructuralExecutionIntent(prompt)).toBe(true);
    expect(hasRecurringAutomationIntent("Check my email for anything unread")).toBe(false);
  });

  it.each([
    [false, false, false],
    [false, true, false],
    [true, false, true],
    [true, true, false],
  ])("maps action=%s receipt=%s to pending=%s", (action, receipt, pending) => {
    expect(requiresPendingNativePostcondition(action, receipt)).toBe(pending);
  });

  it("keeps retrospective explanations conversational", () => {
    const prompt = "Why did you open the browser panel?";
    const answer = "I opened it because I incorrectly interpreted your earlier message.";
    expect(isRetrospectiveNativeActionQuestion(prompt)).toBe(true);
    expect(containsUnverifiedActionClaim(answer)).toBe(true);
    expect(shouldBlockUnverifiedActionClaim(answer, prompt, false, false)).toBe(false);
  });

  it("still blocks completed action claims on action-authorized turns", () => {
    expect(
      shouldBlockUnverifiedActionClaim(
        "I successfully wrote the report to Downloads.",
        "Write report.md to my Downloads folder.",
        true,
        false,
      ),
    ).toBe(true);
  });

  it("accepts a completion claim backed by a verified native execution receipt", () => {
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've created the requested reminder.",
        "Create a reminder for tomorrow.",
        true,
        true,
      ),
    ).toBe(false);
  });

  it.each([
    "I have successfully added the requested reminder.",
    "I've scheduled the hourly Mail check.",
    "I set up the recurring task.",
  ])("recognizes effect completion language that requires a receipt: %s", (claim) => {
    expect(containsUnverifiedActionClaim(claim)).toBe(true);
  });
});

describe("native effect transaction receipts", () => {
  it("keeps a reminder mutation outstanding across an intervening verified read", () => {
    const prompt =
      "Check my reminders, then add a reminder called Auto-Route Verification Test.";
    const routed = nativeEffectExpectationForRouteDecision({
      decision_source: "external_apple_app_write_filter",
      matched_signals: ["explicit Apple app write request"],
    });
    const afterRead = outstandingNativeEffectAfterReceipt(routed, {
      kind: "native_tool",
      effect: "read",
      toolKey: "macos_applescript/read_system_reminders",
      verified: true,
    });
    const bound = bindNativeEffectExpectationToTool(
      afterRead,
      "macos_applescript/add_system_reminder",
      true,
    );

    expect(afterRead).toEqual({ kind: "mutation", toolKey: null });
    expect(shouldBlockUnverifiedActionClaim(
      "I have successfully added the requested reminder.",
      prompt,
      afterRead !== null,
      afterRead === null,
    )).toBe(true);
    expect(outstandingNativeEffectAfterReceipt(bound, {
      kind: "native_tool",
      effect: "mutation",
      toolKey: "macos_applescript/create_system_note",
      verified: true,
    })).toEqual(bound);
    expect(outstandingNativeEffectAfterReceipt(bound, {
      kind: "native_tool",
      effect: "mutation",
      toolKey: "macos_applescript/add_system_reminder",
      verified: true,
    })).toBeNull();
  });
});

describe("native execution intent boundaries", () => {
  it("requires a workflow receipt for a schedule instead of accepting a Mail read receipt", () => {
    const prompt =
      "Read my mail now, then schedule that unread-mail check every hour.";
    const scheduled = nativeEffectExpectationForRouteDecision({
      decision_source: "routine_scheduler_filter",
      matched_signals: ["recurring routine", "routine cadence:v1:1:hour"],
    });

    const afterMailRead = outstandingNativeEffectAfterReceipt(scheduled, {
      kind: "native_tool",
      effect: "read",
      toolKey: "macos_applescript/read_system_emails",
      verified: true,
    });

    expect(afterMailRead).toEqual({ kind: "schedule", toolKey: null });
    expect(shouldBlockUnverifiedActionClaim(
      "I've scheduled the hourly Mail check.",
      prompt,
      afterMailRead !== null,
      afterMailRead === null,
    )).toBe(true);
    expect(outstandingNativeEffectAfterReceipt(scheduled, {
      kind: "workflow_schedule",
      effect: "schedule",
      toolKey: null,
      verified: true,
    })).toBeNull();
  });

  it("does not apply the receipt gate when no executable action was expected", () => {
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've written about that pattern before.",
        "Explain the writing pattern.",
        false,
        false,
      ),
    ).toBe(false);
  });

  it("does not classify questions about prior UI behavior as native task requests", () => {
    expect(
      evaluateLikelyLocalNativeTaskIntent("Why did you open the browser panel?", {
        hasInternalMemoryIntent: false,
        hasLocalPath: false,
        mentionsAppleApp: false,
      }),
    ).toBe(false);
  });
});
