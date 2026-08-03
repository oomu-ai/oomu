import { useEffect, useState } from "react";
import { invoke, isTauriRuntime } from "@/lib/invoke";

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function positiveInteger(value: unknown) {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

export function verifiedStartupModelId(value: unknown): string | null {
  const health = record(value);
  if (!health || health.status !== "ready") return null;
  const modelId = typeof health.classifierModelId === "string"
    ? health.classifierModelId.trim()
    : "";
  const requestedModelId = typeof health.requestedModelId === "string"
    ? health.requestedModelId.trim()
    : "";
  const readiness = positiveInteger(health.readinessGeneration);
  const residency = positiveInteger(health.residencyGeneration);
  const verifiedResidency = positiveInteger(health.verifiedResidencyGeneration);
  const lastVerified = positiveInteger(health.lastVerifiedAtMs);
  if (
    !modelId
    || !requestedModelId
    || !readiness
    || !residency
    || residency !== verifiedResidency
    || !lastVerified
  ) {
    return null;
  }
  return modelId;
}

export function resolvedAgentSessionRoute(
  savedProviderId: string,
  savedModelId: string,
  verifiedStartupModel: string | null,
) {
  if (savedProviderId && savedModelId) {
    return { providerId: savedProviderId, modelId: savedModelId };
  }
  return verifiedStartupModel
    ? { providerId: "local_model", modelId: verifiedStartupModel }
    : { providerId: savedProviderId, modelId: savedModelId };
}

type AgentWithRoute = {
  id: string;
  endpoint?: { provider?: string; modelId?: string };
};

export function resolvedAgentSessionRouteFor(
  agents: AgentWithRoute[],
  agentId: string,
  verifiedStartupModel: string | null,
) {
  const agent = agents.find((entry) => entry.id === agentId);
  return resolvedAgentSessionRoute(
    agent?.endpoint?.provider?.trim() ?? "",
    agent?.endpoint?.modelId?.trim() ?? "",
    verifiedStartupModel,
  );
}

const LOCAL_MODEL_PROVIDER_IDS = new Set([
  "local",
  "local_gemma",
  "local_model",
]);

export function verifiedStartupRouteForAgentEndpoint(
  providerId: string | null | undefined,
  modelId: string | null | undefined,
  verifiedStartupModel: string | null,
) {
  if (!verifiedStartupModel) return null;

  const normalizedProviderId = providerId?.trim().toLowerCase() ?? "";
  const normalizedModelId = modelId?.trim() ?? "";
  const isImplicitRoute = !normalizedProviderId && !normalizedModelId;
  const isMatchingLocalRoute = LOCAL_MODEL_PROVIDER_IDS.has(normalizedProviderId)
    && (!normalizedModelId || normalizedModelId === verifiedStartupModel);

  return isImplicitRoute || isMatchingLocalRoute
    ? { providerId: "local_model", modelId: verifiedStartupModel }
    : null;
}

export function canCreateAgentWithModel(
  name: string,
  selectedModelId: string,
  providerId: string,
  verifiedStartupModel: string | null,
) {
  return Boolean(
    name.trim()
    && (selectedModelId || (providerId === "local_model" && verifiedStartupModel)),
  );
}

export function useVerifiedStartupModel(
  licenseAccepted: boolean | undefined,
  refreshWhenOpened: boolean,
) {
  const [modelId, setModelId] = useState<string | null>(null);

  useEffect(() => {
    if (!licenseAccepted || !isTauriRuntime) {
      return;
    }
    let cancelled = false;
    let refreshTimer: number | null = null;
    const refresh = async () => {
      try {
        const health = await invoke<unknown>("get_auto_route_classifier_health");
        if (!cancelled) setModelId(verifiedStartupModelId(health));
      } catch {
        if (!cancelled) setModelId(null);
      } finally {
        if (!cancelled) {
          refreshTimer = window.setTimeout(refresh, 1_000);
        }
      }
    };
    void refresh();
    return () => {
      cancelled = true;
      if (refreshTimer) window.clearTimeout(refreshTimer);
    };
  }, [licenseAccepted, refreshWhenOpened]);

  return licenseAccepted && isTauriRuntime ? modelId : null;
}
