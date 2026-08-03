import { compactExecutionModelLabel } from "./RoutingIndicator";

type AssistantExecutionMessage = {
  providerId?: string | null;
  modelId?: string | null;
  metadata?: {
    executingProviderId?: string | null;
    targetProviderId?: string | null;
    executingModelId?: string | null;
    targetModelId?: string | null;
  } | null;
};

export function isLocalModelProviderId(providerId: string) {
  const normalized = providerId.trim().toLowerCase().replace(/[\s-]+/g, "_");
  return normalized === "local" || normalized === "local_model" || normalized === "local_gemma";
}

export function assistantExecutionModelLabel(message: AssistantExecutionMessage) {
  const modelId = message.metadata?.executingModelId ??
    message.metadata?.targetModelId ?? message.modelId ?? "";
  if (!modelId) return null;
  return compactExecutionModelLabel(modelId) || null;
}

export function assistantExecutionIsLocal(message: AssistantExecutionMessage) {
  const providerId = (message.metadata?.executingProviderId ??
    message.metadata?.targetProviderId ?? message.providerId ?? "").toLowerCase();
  const modelId = (message.metadata?.executingModelId ??
    message.metadata?.targetModelId ?? message.modelId ?? "").toLowerCase();
  return isLocalModelProviderId(providerId) || providerId.includes("local") ||
    providerId.includes("native") || modelId.includes("gemma") || modelId.includes("local");
}
