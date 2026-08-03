import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { canonicalAssistantDisplayText } from "./canonicalAssistantDisplay";
import { clearTurnRecovery } from "./turnRecoveryPersistence";

export type ChatSubmissionOutcome = { accepted: boolean };

export const ACCEPTED_CHAT_SUBMISSION: ChatSubmissionOutcome = { accepted: true };
export const REJECTED_CHAT_SUBMISSION: ChatSubmissionOutcome = { accepted: false };

type AcceptedChatTurn = {
  turnId: string;
  messageId: number;
  accepted: boolean;
};

export async function acceptDurableChatTurn(
  turn: ChatTurnContext,
  message: string,
  resumeInterrupted = false,
) {
  const acceptedMessage = message.trim() ? message : "Please review the attached file.";
  const accepted = await invoke<AcceptedChatTurn>(
    resumeInterrupted ? "resume_interrupted_chat_turn" : "accept_chat_turn",
    {
      request: {
        turn_id: turn.turnId,
        generation_token: turn.generationToken,
        parent_turn_id: turn.ancestry.parentTurnId,
        root_turn_id: turn.ancestry.rootTurnId,
        turn_kind: turn.ancestry.kind,
        session_id: turn.sessionId,
        agent_id: turn.agentId,
        provider_id: turn.route.providerId,
        model_id: turn.route.modelId,
        message: acceptedMessage,
      },
    },
  );
  if (!accepted.accepted || accepted.turnId !== turn.turnId || accepted.messageId <= 0) {
    throw new Error("chat_turn_acceptance_failed");
  }
  return accepted;
}

export async function finalizeDurableChatTurn(
  turn: ChatTurnContext,
  result: { role: "assistant" | "system"; content: string; status: "completed" | "failed" | "cancelled" | "escalated" },
) {
  const content = result.role === "assistant"
    ? canonicalAssistantDisplayText(result.content)
    : result.content;
  const messageId = await invoke<number>("finalize_accepted_chat_turn", {
    request: {
      turn_id: turn.turnId,
      generation_token: turn.generationToken,
      parent_turn_id: turn.ancestry.parentTurnId,
      root_turn_id: turn.ancestry.rootTurnId,
      turn_kind: turn.ancestry.kind,
      session_id: turn.sessionId,
      agent_id: turn.agentId,
      provider_id: turn.route.providerId,
      model_id: turn.route.modelId,
      ...result,
      content,
    },
  });
  const recoveryIdentity = {
    sessionId: turn.sessionId,
    rootTurnId: turn.ancestry.rootTurnId,
    turnId: turn.turnId,
    generationToken: turn.generationToken,
  };
  clearTurnRecovery(recoveryIdentity, "auto_route");
  clearTurnRecovery(recoveryIdentity, "apple_permission");
  return messageId;
}

export async function abandonDurableChatTurn(
  turn: ChatTurnContext,
  content: string,
) {
  return invoke<number | null>("abandon_accepted_chat_turn", {
    request: {
      turn_id: turn.turnId,
      generation_token: turn.generationToken,
      parent_turn_id: turn.ancestry.parentTurnId,
      root_turn_id: turn.ancestry.rootTurnId,
      turn_kind: turn.ancestry.kind,
      session_id: turn.sessionId,
      agent_id: turn.agentId,
      provider_id: turn.route.providerId,
      model_id: turn.route.modelId,
      content,
    },
  });
}
