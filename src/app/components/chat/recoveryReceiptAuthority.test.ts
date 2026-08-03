import { describe, expect, it } from "vitest";
import type { AgentExecutionRecoveryState } from "./agentExecutionRecovery";
import {
  recoveryReceiptAuthoritiesForTranscript,
  terminalRecoveryProjectionRefreshKey,
} from "./recoveryReceiptAuthority";

const digest = "a".repeat(64);

function recoveryState(
  overrides: Partial<AgentExecutionRecoveryState> = {},
): AgentExecutionRecoveryState {
  return {
    executionId: "execution-7",
    planId: "plan-7",
    status: "halted",
    terminalPhase: "restart_recovery_ready",
    terminalVerified: false,
    verifiedComplete: false,
    ...overrides,
  };
}

function receipt(
  messageId: number,
  overrides: Partial<{
    executionId: string;
    planId: string;
    recoverable: boolean;
    recoveryAction: string;
    code: string;
    nextOperation: string | null;
    frozenArgumentSha256: string | null;
  }> = {},
) {
  const {
    nextOperation = "draft_release_recovery_email",
    frozenArgumentSha256 = digest,
    ...receiptOverrides
  } = overrides;
  return {
    messageId,
    receipt: {
      executionId: "execution-7",
      planId: "plan-7",
      recoverable: true,
      recoveryAction: "resume_same_execution",
      code: "agent_execution_interrupted",
      ...receiptOverrides,
      context: {
        nextOperation,
        frozenArgumentSha256,
      },
    },
  };
}

describe("recovery receipt authority", () => {
  it("makes the current Mail interruption actionable and the historical Calendar receipt inert", () => {
    const entries = [
      receipt(41, {
        code: "calendar_target_resolved",
        nextOperation: "create_release_recovery_calendar_event",
        frozenArgumentSha256: null,
      }),
      receipt(52),
    ];
    const state = recoveryState();

    const authorities = recoveryReceiptAuthoritiesForTranscript(
      entries,
      "ready",
      new Map([[state.executionId, state]]),
    );

    expect(authorities.get(41)).toBe("inactive");
    expect(authorities.get(52)).toBe("current");
  });

  it("never authorizes a restart-ready receipt without the frozen Mail operation", () => {
    const state = recoveryState();
    const authorities = recoveryReceiptAuthoritiesForTranscript(
      [receipt(52, { frozenArgumentSha256: "not-a-digest" })],
      "ready",
      new Map([[state.executionId, state]]),
    );

    expect(authorities.get(52)).toBe("inactive");
  });

  it("maps projection lifecycle states without granting action authority", () => {
    const entries = [receipt(52), receipt(53, { recoverable: false })];

    expect(recoveryReceiptAuthoritiesForTranscript(entries, "loading", new Map()).get(52))
      .toBe("checking");
    expect(recoveryReceiptAuthoritiesForTranscript(entries, "failed", new Map()).get(52))
      .toBe("unavailable");
    expect(recoveryReceiptAuthoritiesForTranscript(entries, "ready", new Map()).get(52))
      .toBe("inactive");
    expect(recoveryReceiptAuthoritiesForTranscript(entries, "loading", new Map()).has(53))
      .toBe(false);
  });

  it("keys only terminal execution states for an explicit projection refresh", () => {
    expect(terminalRecoveryProjectionRefreshKey({
      sessionId: "session-7",
      executionId: "execution-7",
      status: "running",
    })).toBeNull();
    expect(terminalRecoveryProjectionRefreshKey({
      sessionId: "session-7",
      executionId: "execution-7",
      status: "halted",
    })).toBe('["session-7","execution-7","halted"]');
  });
});
