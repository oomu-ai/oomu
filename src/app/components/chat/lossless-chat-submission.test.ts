import { beforeEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import {
  abandonDurableChatTurn,
  acceptDurableChatTurn,
} from "./submissionAcceptance";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const turn = createChatTurnContext({
  turnId: "turn-lossless",
  generationToken: "generation-lossless",
  sessionId: "session-lossless",
  agentId: "agent-lossless",
  route: {
    providerId: "local_model",
    modelId: "test-model",
    dynamicRoutingEnabled: true,
    automatedWebGroundingEnabled: false,
  },
});

describe("lossless chat submission", () => {
  beforeEach(() => invokeMock.mockReset());

  it("hands the exact untrimmed prompt and immutable turn identity to native acceptance", async () => {
    invokeMock.mockResolvedValue({
      turnId: turn.turnId,
      messageId: 41,
      accepted: true,
    });
    const message = "  Compare /Users/example/plan.md, then explain the boundary.  ";

    await expect(acceptDurableChatTurn(turn, message)).resolves.toMatchObject({
      messageId: 41,
      accepted: true,
    });
    expect(invokeMock).toHaveBeenCalledWith("accept_chat_turn", {
      request: expect.objectContaining({
        turn_id: turn.turnId,
        generation_token: turn.generationToken,
        session_id: turn.sessionId,
        message,
      }),
    });
  });

  it("does not acknowledge a malformed or rejected native acceptance", async () => {
    invokeMock.mockResolvedValue({
      turnId: "different-turn",
      messageId: 0,
      accepted: false,
    });

    await expect(acceptDurableChatTurn(turn, "Keep this exact draft"))
      .rejects.toThrow("chat_turn_acceptance_failed");
  });

  it("uses a canonical nonempty message for an attachment-only turn", async () => {
    invokeMock.mockResolvedValue({ turnId: turn.turnId, messageId: 51, accepted: true });
    await acceptDurableChatTurn(turn, "   ");
    expect(invokeMock).toHaveBeenCalledWith("accept_chat_turn", {
      request: expect.objectContaining({
        turn_id: turn.turnId,
        message: "Please review the attached file.",
      }),
    });
  });

  it("uses the atomic native resume command only for an interrupted turn", async () => {
    invokeMock.mockResolvedValue({ turnId: turn.turnId, messageId: 41, accepted: true });

    await acceptDurableChatTurn(turn, "Keep this exact draft", true);

    expect(invokeMock).toHaveBeenCalledWith("resume_interrupted_chat_turn", {
      request: expect.objectContaining({
        turn_id: turn.turnId,
        generation_token: turn.generationToken,
        provider_id: turn.route.providerId,
        model_id: turn.route.modelId,
        message: "Keep this exact draft",
      }),
    });
  });

  it("abandons only the exact immutable accepted turn context", async () => {
    invokeMock.mockResolvedValue(52);

    await expect(
      abandonDurableChatTurn(
        turn,
        "OOMU couldn't start this reply safely. Try again.",
      ),
    ).resolves.toBe(52);
    expect(invokeMock).toHaveBeenCalledWith("abandon_accepted_chat_turn", {
      request: {
        turn_id: turn.turnId,
        generation_token: turn.generationToken,
        parent_turn_id: null,
        root_turn_id: turn.turnId,
        turn_kind: "root",
        session_id: turn.sessionId,
        agent_id: turn.agentId,
        provider_id: "local_model",
        model_id: "test-model",
        content: "OOMU couldn't start this reply safely. Try again.",
      },
    });
  });
});
