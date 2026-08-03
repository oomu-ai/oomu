import { beforeEach, describe, expect, it } from "vitest";
import type { AutoRouteAttention } from "./AutoRouteAttentionCard";
import {
  clearTurnRecovery,
  clearTerminalTurnRecoveries,
  persistTurnRecovery,
  readTurnRecoveries,
  readTurnRecovery,
  turnRecoveryIdentityKey,
} from "./turnRecoveryPersistence";

function attention(): AutoRouteAttention {
  return {
    sessionId: "session-301",
    rootTurnId: "turn-301",
    turnId: "turn-301",
    generationToken: "generation-301",
    localProviderId: "local_model",
    localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    recommendedLocalProviderId: "local_model",
    recommendedLocalModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    cloudModelId: "gemini-3.5-flash",
    failureCode: "classifier_inference_timeout",
    failureBoundary: "auto_route_classifier_inference",
    kind: "timeout",
  };
}

describe("durable frontend turn recovery", () => {
  beforeEach(() => window.localStorage.clear());

  it("restores only bounded recovery metadata and never stores prompt contents", () => {
    const value = attention();
    expect(persistTurnRecovery({
      type: "auto_route",
      sessionId: value.sessionId,
      rootTurnId: value.rootTurnId,
      turnId: value.turnId,
      generationToken: value.generationToken,
      attention: value,
      updatedAtMs: Date.now(),
    })).toBe(true);

    expect(readTurnRecovery("session-301", "auto_route")?.attention).toEqual(value);
    const raw = window.localStorage.getItem("oomu.chat.turn-recovery.v1") ?? "";
    expect(raw).not.toContain("private calendar contents");
    expect(raw).not.toContain("message");
  });

  it("keeps Auto-route and Apple permission recovery independently addressable", () => {
    const value = attention();
    persistTurnRecovery({
      type: "auto_route",
      ...value,
      attention: value,
      updatedAtMs: Date.now(),
    });
    persistTurnRecovery({
      type: "apple_permission",
      sessionId: value.sessionId,
      rootTurnId: value.rootTurnId,
      turnId: value.turnId,
      generationToken: value.generationToken,
      boundary: "direct_apple_read",
      code: "calendar_permission_denied",
      descriptor: { capabilityId: "calendar", state: "denied" },
      updatedAtMs: Date.now(),
    });

    expect(readTurnRecoveries()).toHaveLength(2);
    expect(clearTurnRecovery(value, "auto_route")).toBe(true);
    expect(readTurnRecovery(value.sessionId, "auto_route")).toBeNull();
    expect(readTurnRecovery(value.sessionId, "apple_permission")?.descriptor)
      .toEqual({ capabilityId: "calendar", state: "denied" });
  });

  it("rejects malformed or expired records instead of reviving stale work", () => {
    window.localStorage.setItem("oomu.chat.turn-recovery.v1", JSON.stringify({
      schema: "oomu.chat.turn_recovery.v1",
      records: [{
        type: "apple_permission",
        sessionId: "session-301",
        rootTurnId: "turn-301",
        turnId: "turn-301",
        generationToken: "generation-301",
        boundary: "private Calendar title",
        code: "calendar_permission_denied",
        descriptor: { capabilityId: "calendar", state: "denied" },
        updatedAtMs: 1,
      }],
    }));
    expect(readTurnRecoveries()).toEqual([]);
  });

  it("keeps paused work until its exact terminal result is durable", () => {
    const value = attention();
    persistTurnRecovery({
      type: "auto_route",
      ...value,
      attention: value,
      updatedAtMs: Date.now(),
    });

    expect(clearTerminalTurnRecoveries(value.sessionId, new Set([
      turnRecoveryIdentityKey({ ...value, turnId: "another-turn" }),
    ]))).toBe(true);
    expect(readTurnRecovery(value.sessionId, "auto_route")).not.toBeNull();
    expect(clearTerminalTurnRecoveries(value.sessionId, new Set([
      turnRecoveryIdentityKey(value),
    ]))).toBe(true);
    expect(readTurnRecovery(value.sessionId, "auto_route")).toBeNull();
  });

  it("reports a storage failure instead of pretending recovery is durable", () => {
    const value = attention();
    const failingStorage = {
      getItem: () => null,
      setItem: () => { throw new Error("storage unavailable"); },
    } as unknown as Storage;
    expect(persistTurnRecovery({
      type: "auto_route",
      ...value,
      attention: value,
      updatedAtMs: Date.now(),
    }, failingStorage)).toBe(false);
  });

});

describe("Sprint 304 exact turn recovery identity", () => {
  beforeEach(() => window.localStorage.clear());

  it("hydrates only the exact session, root turn, failed turn, and generation", () => {
    const first = attention();
    const second = {
      ...first,
      sessionId: "session-302",
      rootTurnId: "turn-302",
      turnId: "turn-302-failed",
      generationToken: "generation-302",
    };
    for (const value of [first, second]) {
      persistTurnRecovery({
        type: "auto_route",
        ...value,
        attention: value,
        updatedAtMs: Date.now(),
      });
    }

    expect(readTurnRecovery(first.sessionId, "auto_route")?.attention).toEqual(first);
    expect(readTurnRecovery(second.sessionId, "auto_route")?.attention).toEqual(second);
    clearTurnRecovery(first, "auto_route");
    expect(readTurnRecovery(first.sessionId, "auto_route")).toBeNull();
    expect(readTurnRecovery(second.sessionId, "auto_route")?.attention).toEqual(second);
  });
});
