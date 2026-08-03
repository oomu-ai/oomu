import {
  createChatTurnContext,
  type ChatTurnContext,
} from "@/lib/chatTurnContext";
import type { PersistedAcceptedTurnIdentity } from "./usePersistedTurnReplay";

type ChatTurnContextInput = Parameters<typeof createChatTurnContext>[0];

export function createReplayAwareTurnContext(
  resume: PersistedAcceptedTurnIdentity | undefined,
  fallback: ChatTurnContextInput,
): ChatTurnContext {
  if (!resume) return createChatTurnContext(fallback);
  return createChatTurnContext({
    ...fallback,
    turnId: resume.turnId,
    generationToken: resume.generationToken,
    ancestry: {
      ...fallback.ancestry,
      rootTurnId: resume.rootTurnId,
    },
    route: {
      ...fallback.route,
      providerId: resume.providerId,
      modelId: resume.modelId,
      dynamicRoutingEnabled: resume.dynamicRoutingEnabled
        || (resume.providerId === "dynamic" && resume.modelId === "dynamic"),
    },
  });
}
