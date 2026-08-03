import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@/lib/invoke";
import { finalizeDurableChatTurn } from "./submissionAcceptance";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { persistTurnRecovery, readTurnRecovery } from "./turnRecoveryPersistence";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const turn = {
  turnId: "turn-1",
  generationToken: "generation-1",
  sessionId: "session-1",
  agentId: "agent-1",
  projectId: null,
  ancestry: { parentTurnId: null, rootTurnId: "turn-1", kind: "root" },
  route: {
    providerId: "provider-1",
    modelId: "model-1",
    dynamicRoutingEnabled: false,
    automatedWebGroundingEnabled: false,
  },
  attachmentGrants: [],
  createdAtMs: 1,
} satisfies ChatTurnContext;

describe("finalizeDurableChatTurn", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    window.localStorage.clear();
  });

  it("removes internal assistant envelopes before durable persistence", async () => {
    vi.mocked(invoke).mockResolvedValue(1);

    await finalizeDurableChatTurn(turn, {
      role: "assistant",
      content: "Ready.\n<tool_call>{\"name\":\"read_file\"}</tool_call>",
      status: "completed",
    });

    expect(invoke).toHaveBeenCalledWith("finalize_accepted_chat_turn", {
      request: expect.objectContaining({ content: "Ready." }),
    });
  });

  it("clears paused recovery only after native terminal persistence succeeds", async () => {
    persistTurnRecovery({
      type: "auto_route",
      sessionId: turn.sessionId,
      rootTurnId: turn.ancestry.rootTurnId,
      turnId: turn.turnId,
      generationToken: turn.generationToken,
      attention: {
        sessionId: turn.sessionId,
        rootTurnId: turn.ancestry.rootTurnId,
        turnId: turn.turnId,
        generationToken: turn.generationToken,
        localProviderId: "local_model",
        localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        recommendedLocalProviderId: "local_model",
        recommendedLocalModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
        cloudModelId: "",
        failureCode: "classifier_not_ready",
        failureBoundary: "auto_route_classifier",
        kind: "preparing",
      },
      updatedAtMs: Date.now(),
    });
    vi.mocked(invoke).mockRejectedValueOnce(new Error("database unavailable"));
    await expect(finalizeDurableChatTurn(turn, {
      role: "system", content: "Stopped safely.", status: "failed",
    })).rejects.toThrow("database unavailable");
    expect(readTurnRecovery(turn.sessionId, "auto_route")).not.toBeNull();

    vi.mocked(invoke).mockResolvedValueOnce(7);
    await finalizeDurableChatTurn(turn, {
      role: "system", content: "Stopped safely.", status: "failed",
    });
    expect(readTurnRecovery(turn.sessionId, "auto_route")).toBeNull();
  });
});
