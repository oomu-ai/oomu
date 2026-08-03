import type { ConfiguredProvider } from "@/lib/modelRegistry";
import type { ChatTurnContext } from "@/lib/chatTurnContext";

export function planningPreferenceForProvider(
  configuredProviders: ConfiguredProvider[],
  providerId: string,
) {
  const normalized = (configuredProviders.find((entry) => entry.id === providerId)?.providerId ?? providerId)
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
  if (
    normalized === "gemini_pro" ||
    normalized === "gemini" ||
    normalized === "google" ||
    normalized === "google_gemini"
  ) {
    return "gemini_pro";
  }
  if (normalized === "chat_gpt" || normalized === "chatgpt" || normalized === "openai") {
    return "chat_gpt";
  }
  return "local_gemma";
}

export function plannerRequestRoute(
  configuredProviders: ConfiguredProvider[],
  route: ChatTurnContext["route"],
) {
  const configuredProvider = configuredProviders.find((entry) => entry.id === route.providerId);
  const providerFamily = (configuredProvider?.providerId ?? route.providerId)
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
  const selectedProviderId =
    providerFamily === "local" ||
    providerFamily === "local_model" ||
    providerFamily === "local_gemma"
      ? "local_model"
      : route.providerId;
  return {
    selected_model: planningPreferenceForProvider(configuredProviders, route.providerId),
    selected_provider_id: selectedProviderId,
    selected_model_id: route.modelId,
    dynamic_routing_enabled: route.dynamicRoutingEnabled,
  };
}
