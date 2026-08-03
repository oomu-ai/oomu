import { describe, expect, it } from "vitest";
import { interruptedTurnAttentionForSession } from "./useChatTurnRecovery";

describe("Sprint 304 visible interrupted-turn recovery", () => {
  it("projects only the active session's exact durable turn identity", () => {
    expect(interruptedTurnAttentionForSession("chat-a", [{
      role: "user",
      content: "Keep this exact request.",
      providerId: "dynamic",
      modelId: "dynamic",
      metadata: {
        turnId: "turn-a",
        rootTurnId: "root-a",
        generationToken: "generation-a",
        turnState: "interrupted",
      },
    }])).toMatchObject({
      sessionId: "chat-a",
      rootTurnId: "root-a",
      turnId: "turn-a",
      generationToken: "generation-a",
      kind: "interrupted",
      failureCode: "turn_interrupted",
    });
    expect(interruptedTurnAttentionForSession("chat-b", [])).toBeNull();
  });

  it("does not duplicate Apple permission recovery for an interrupted permission turn", () => {
    expect(interruptedTurnAttentionForSession("chat-a", [{
      role: "user",
      content: "Use Calendar.",
      metadata: {
        turnId: "turn-permission",
        generationToken: "generation-permission",
        turnState: "interrupted",
        permissionContinuation: {
          state: "waiting",
          capabilityId: "calendar",
        },
      },
    }])).toBeNull();
  });
});
