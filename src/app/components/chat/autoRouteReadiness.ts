import { useEffect, useState } from "react";
import { invoke, isTauriRuntime } from "@/lib/invoke";

export type LocalModelStatus =
  | "cold"
  | "loading"
  | "ready"
  | "degraded"
  | "shutdown"
  | "unknown";

export type AutoRouteReadinessStatus =
  | "loading"
  | "ready"
  | "recovering"
  | "degraded"
  | "shutdown"
  | "unknown";

export type AutoRouteSessionReadiness = {
  status: AutoRouteReadinessStatus;
  sessionId: string;
  dynamicBindingValid: boolean;
  classifierModelId: string | null;
  classifierReady: boolean;
  localProviderId: string | null;
  localProviderType: string | null;
  localModelId: string | null;
  routeGeneration: number;
  localModelReady: boolean;
  recommendedLocalProviderId: string | null;
  recommendedLocalModelId: string | null;
  contextBudgetValid: boolean;
  cloudTargetRequired: boolean;
  cloudTargetReady: boolean;
  storageReady: boolean;
  auditReady: boolean;
  readinessGeneration: number;
  lastVerifiedAtMs: number | null;
  failureCode: string | null;
  failureBoundary: string | null;
};

type AutoRouteReadinessOptions = {
  sessionId: string;
  dynamicRoutingEnabled: boolean;
  localModelId: string;
  refreshKey?: string;
};

const TRANSITIONAL_READINESS_REFRESH_MS = 5_000;

const readinessStatuses = new Set<AutoRouteReadinessStatus>([
  "loading",
  "ready",
  "recovering",
  "degraded",
  "shutdown",
  "unknown",
]);

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function optionalString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function strictBoolean(value: unknown) {
  return value === true;
}

function nonNegativeInteger(value: unknown) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : 0;
}

function optionalTimestamp(value: unknown) {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

export function normalizeLocalModelStatus(value: unknown): LocalModelStatus {
  const rawStatus = value && typeof value === "object" && "status" in value
    ? (value as { status?: unknown }).status
    : value;
  switch (String(rawStatus ?? "").trim().toLowerCase()) {
    case "cold":
      return "cold";
    case "loading":
      return "loading";
    case "ready":
      return "ready";
    case "degraded":
      return "degraded";
    case "shutdown":
      return "shutdown";
    default:
      return "unknown";
  }
}

export function normalizeAutoRouteSessionReadiness(
  value: unknown,
  requestedSessionId: string,
): AutoRouteSessionReadiness {
  const candidate = record(value);
  if (!candidate) {
    return unavailableReadiness(requestedSessionId);
  }
  const sessionId = optionalString(candidate.sessionId) ?? "";
  const rawStatus = String(candidate.status ?? "").trim().toLowerCase();
  const reportedStatus = readinessStatuses.has(rawStatus as AutoRouteReadinessStatus)
    ? rawStatus as AutoRouteReadinessStatus
    : "unknown";
  const readiness: AutoRouteSessionReadiness = {
    status: reportedStatus,
    sessionId,
    dynamicBindingValid: strictBoolean(candidate.dynamicBindingValid),
    classifierModelId: optionalString(candidate.classifierModelId),
    classifierReady: strictBoolean(candidate.classifierReady),
    localProviderId: optionalString(candidate.localProviderId),
    localProviderType: optionalString(candidate.localProviderType),
    localModelId: optionalString(candidate.localModelId),
    routeGeneration: nonNegativeInteger(candidate.routeGeneration),
    localModelReady: strictBoolean(candidate.localModelReady),
    recommendedLocalProviderId: optionalString(candidate.recommendedLocalProviderId),
    recommendedLocalModelId: optionalString(candidate.recommendedLocalModelId),
    contextBudgetValid: strictBoolean(candidate.contextBudgetValid),
    cloudTargetRequired: strictBoolean(candidate.cloudTargetRequired),
    cloudTargetReady: strictBoolean(candidate.cloudTargetReady),
    storageReady: strictBoolean(candidate.storageReady),
    auditReady: strictBoolean(candidate.auditReady),
    readinessGeneration: nonNegativeInteger(candidate.readinessGeneration),
    lastVerifiedAtMs: optionalTimestamp(candidate.lastVerifiedAtMs),
    failureCode: optionalString(candidate.failureCode),
    failureBoundary: optionalString(candidate.failureBoundary),
  };
  const snapshotMatchesSession = Boolean(requestedSessionId) && sessionId === requestedSessionId;
  const cloudReady = !readiness.cloudTargetRequired || readiness.cloudTargetReady;
  const allReady = snapshotMatchesSession
    && readiness.dynamicBindingValid
    && readiness.classifierReady
    && Boolean(readiness.classifierModelId)
    && readiness.localModelReady
    && Boolean(readiness.localProviderId)
    && Boolean(readiness.localProviderType)
    && Boolean(readiness.localModelId)
    && readiness.routeGeneration > 0
    && readiness.contextBudgetValid
    && cloudReady
    && readiness.storageReady
    && readiness.auditReady
    && readiness.readinessGeneration > 0
    && readiness.lastVerifiedAtMs !== null;

  // A native "ready" label is not enough. The frontend only repeats it when
  // the complete, current-session snapshot proves every required precondition.
  if (readiness.status === "ready" && !allReady) {
    readiness.status = "degraded";
  }
  return readiness;
}

function unavailableReadiness(sessionId: string): AutoRouteSessionReadiness {
  return {
    status: "unknown",
    sessionId,
    dynamicBindingValid: false,
    classifierModelId: null,
    classifierReady: false,
    localProviderId: null,
    localProviderType: null,
    localModelId: null,
    routeGeneration: 0,
    localModelReady: false,
    recommendedLocalProviderId: null,
    recommendedLocalModelId: null,
    contextBudgetValid: false,
    cloudTargetRequired: false,
    cloudTargetReady: false,
    storageReady: false,
    auditReady: false,
    readinessGeneration: 0,
    lastVerifiedAtMs: null,
    failureCode: null,
    failureBoundary: null,
  };
}

export function currentSessionAutoRouteReadiness(
  readiness: AutoRouteSessionReadiness,
  sessionId: string,
  dynamicRoutingEnabled: boolean,
) {
  if (readiness.sessionId === sessionId) return readiness;
  return dynamicRoutingEnabled && sessionId
    ? { ...unavailableReadiness(sessionId), status: "loading" as const }
    : unavailableReadiness(sessionId);
}

export function useAutoRouteReadiness({
  sessionId,
  dynamicRoutingEnabled,
  localModelId,
  refreshKey = "",
}: AutoRouteReadinessOptions) {
  const [localGeneration, setLocalGeneration] = useState<{
    modelId: string;
    status: LocalModelStatus;
  }>(() => ({
    modelId: localModelId,
    // Preserve the established manual-chat behavior: native execution remains
    // the authority if the first health poll has not returned yet.
    status: "ready",
  }));
  const [sessionReadiness, setSessionReadiness] = useState<AutoRouteSessionReadiness>(
    () => dynamicRoutingEnabled
      ? { ...unavailableReadiness(sessionId), status: "loading" }
      : unavailableReadiness(sessionId),
  );
  const localModelStatus = localGeneration.modelId === localModelId
    ? localGeneration.status
    : "loading";

  useEffect(() => {
    if (!isTauriRuntime) {
      return;
    }

    let cancelled = false;
    let requestInFlight = false;
    let refreshTimerId: number | null = null;

    function scheduleTransitionalRefresh() {
      if (cancelled || refreshTimerId !== null) return;
      refreshTimerId = window.setTimeout(() => {
        refreshTimerId = null;
        void refreshReadiness();
      }, TRANSITIONAL_READINESS_REFRESH_MS);
    }

    async function refreshReadiness() {
      if (requestInFlight) return;
      if (refreshTimerId !== null) {
        window.clearTimeout(refreshTimerId);
        refreshTimerId = null;
      }
      requestInFlight = true;
      const generationPromise = invoke<unknown>("get_local_generation_health", {
        modelId: localModelId || null,
        model_id: localModelId || null,
      });
      const sessionPromise = dynamicRoutingEnabled && sessionId
        ? invoke<unknown>("get_auto_route_session_readiness", { sessionId })
        : Promise.resolve(null);
      try {
        const [generationStatus, currentSessionReadiness] = await Promise.allSettled([
          generationPromise,
          sessionPromise,
        ]);
        if (cancelled) {
          return;
        }
        const nextLocalStatus = generationStatus.status === "fulfilled"
          ? normalizeLocalModelStatus(generationStatus.value)
          : "unknown";
        const nextSessionReadiness = !dynamicRoutingEnabled || !sessionId
          ? unavailableReadiness(sessionId)
          : currentSessionReadiness.status === "fulfilled"
            ? normalizeAutoRouteSessionReadiness(currentSessionReadiness.value, sessionId)
            : unavailableReadiness(sessionId);
        setLocalGeneration({
          modelId: localModelId,
          status: nextLocalStatus,
        });
        setSessionReadiness(nextSessionReadiness);
        const readinessIsTransitioning = dynamicRoutingEnabled
          && Boolean(sessionId)
          && ["loading", "recovering"].includes(nextSessionReadiness.status);
        if (nextLocalStatus === "loading" || readinessIsTransitioning) {
          scheduleTransitionalRefresh();
        }
      } finally {
        requestInFlight = false;
      }
    }

    function refreshWhenVisible() {
      if (document.visibilityState === "visible") {
        void refreshReadiness();
      }
    }

    void refreshReadiness();
    window.addEventListener("focus", refreshWhenVisible);
    document.addEventListener("visibilitychange", refreshWhenVisible);

    return () => {
      cancelled = true;
      if (refreshTimerId !== null) {
        window.clearTimeout(refreshTimerId);
      }
      window.removeEventListener("focus", refreshWhenVisible);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
  }, [dynamicRoutingEnabled, localModelId, refreshKey, sessionId]);

  return {
    localModelStatus,
    autoRouteSessionReadiness: currentSessionAutoRouteReadiness(
      sessionReadiness,
      sessionId,
      dynamicRoutingEnabled,
    ),
  };
}
