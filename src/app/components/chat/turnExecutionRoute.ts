import {
  rebindChatTurnExecutionRoute,
  type ChatTurnContext,
} from "@/lib/chatTurnContext";
import { normalizeChatMessageMetadata } from "./messageMetadata";

export function executionTurnContextFromPlanReceipt(
  context: ChatTurnContext,
  metadataValue: unknown,
) {
  const metadata = normalizeChatMessageMetadata(
    metadataValue,
    context.route.providerId,
    context.route.modelId,
  );
  return rebindChatTurnExecutionRoute(
    context,
    metadata?.executingProviderId ?? context.route.providerId,
    metadata?.executingModelId ?? context.route.modelId,
  );
}
