import type { ChatSession } from "@/lib/chatSessions";
import {
  canonicalModelId,
  modelsForProvider,
  providerConfigurationId,
  providerTypeId,
  resolveReasoningFallback,
  supportedReasoningLevelsForModel,
  type ConfiguredProvider,
  type ReasoningLevel,
} from "@/lib/modelRegistry";

import { isLocalModelProviderId } from "./assistantExecutionMetadata";
import { isDynamicRouteId } from "./chatPresentationHelpers";
import type { AutoRouteBaselineIpc } from "./useAutoRouteActivation";
import type { RouteOverride } from "./sessionRouting";

export function buildAutoRouteBaseline(
  route: RouteOverride,
  supportedReasoningLevels: ReasoningLevel[],
  contextBudget: number,
): AutoRouteBaselineIpc {
  return {
    providerConfigId: providerConfigurationId(route.providerId),
    providerType: providerTypeId(route.providerType),
    modelId: canonicalModelId(route.modelId),
    reasoningDepth: resolveReasoningFallback(route.reasoning, supportedReasoningLevels),
    contextBudget,
  };
}

export function sessionUsesDynamicBinding(session: ChatSession | null | undefined) {
  return isDynamicRouteId(session?.providerId) && isDynamicRouteId(session?.modelId);
}

export function legacySessionConfigWriteAllowed(
  session: ChatSession | null | undefined,
  dynamicRoutingActive = false,
) {
  if (dynamicRoutingActive) return false;
  if (!sessionUsesDynamicBinding(session)) return true;
  return session?.dynamicRoutingOverride === false;
}

export function persistLegacySessionConfigIfAllowed(
  persist: (sessionId: string, route: RouteOverride) => unknown,
  sessions: ChatSession[],
  sessionId: string,
  blocked: boolean,
  route: RouteOverride,
  reasoning = route.reasoning,
  context = route.context,
) {
  const session = sessions.find((candidate) => candidate.id === sessionId);
  if (blocked || !legacySessionConfigWriteAllowed(session)) return false;
  void persist(sessionId, { ...route, reasoning, context });
  return true;
}

export type SessionConfigRecord = {
  sessionId?: string;
  session_id?: string;
  reasoningDepth?: string;
  reasoning_depth?: string;
  contextBudget?: number;
  context_budget?: number;
  modelId?: string | null;
  model_id?: string | null;
  localProviderConfigId?: string | null;
  local_provider_config_id?: string | null;
  localProviderType?: string | null;
  local_provider_type?: string | null;
  localRouteGeneration?: number;
  local_route_generation?: number;
};

export function sessionConfigReasoning(config: SessionConfigRecord | null) {
  return config?.reasoningDepth ?? config?.reasoning_depth ?? null;
}

export function sessionConfigContextBudget(config: SessionConfigRecord | null) {
  const value = config?.contextBudget ?? config?.context_budget;
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function sessionConfigLocalProviderId(config: SessionConfigRecord | null) {
  return config?.localProviderConfigId?.trim() || config?.local_provider_config_id?.trim() || "";
}

export function sessionConfigLocalProviderType(config: SessionConfigRecord | null) {
  return config?.localProviderType?.trim() || config?.local_provider_type?.trim() || "";
}

export function sessionConfigModelId(config: SessionConfigRecord | null) {
  return config?.modelId?.trim() || config?.model_id?.trim() || "";
}

export function authoritativeSessionConfigRouteIdentity(config: SessionConfigRecord | null) {
  const providerConfigId = sessionConfigLocalProviderId(config);
  const providerType = sessionConfigLocalProviderType(config);
  const modelId = sessionConfigModelId(config);
  const generation = config?.localRouteGeneration ?? config?.local_route_generation;
  if (
    !providerConfigId
    || !providerType
    || !modelId
    || typeof generation !== "number"
    || !Number.isInteger(generation)
    || generation <= 0
  ) {
    return null;
  }
  return {
    providerConfigId: providerConfigurationId(providerConfigId),
    providerType: providerTypeId(providerType),
    modelId: canonicalModelId(modelId),
    generation,
  };
}

export function providerClassIdForRoute(
  configuredProviders: ConfiguredProvider[],
  routeProviderId: string,
) {
  const provider = configuredProviders.find((entry) => entry.id === routeProviderId);
  return provider?.providerId ?? routeProviderId;
}

export function typedProviderClassIdForRoute(
  configuredProviders: ConfiguredProvider[],
  routeProviderId: string,
) {
  return providerTypeId(providerClassIdForRoute(configuredProviders, routeProviderId));
}

export function routeUsesLocalModel(
  configuredProviders: ConfiguredProvider[],
  routeProviderId: string,
) {
  return !routeProviderId.trim()
    || isLocalModelProviderId(providerClassIdForRoute(configuredProviders, routeProviderId));
}

export function supportedReasoningLevelsForRoute(
  configuredProviders: ConfiguredProvider[],
  providerId: string,
  modelId: string,
) {
  const model = modelsForProvider(configuredProviders, providerId)
    .find((entry) => entry.modelId === modelId);
  return model?.supportedReasoningLevels
    ?? supportedReasoningLevelsForModel(providerClassIdForRoute(configuredProviders, providerId), modelId);
}
