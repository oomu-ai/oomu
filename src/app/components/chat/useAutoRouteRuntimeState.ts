import { useEffect, useMemo } from "react";
import {
  configuredProviderIsRunnable,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import type {
  AutoRouteAttention,
  AutoRouteRecoveryAction,
} from "./AutoRouteAttentionCard";
import { useAutoRouteReadiness } from "./autoRouteReadiness";

type AutoRouteRuntimeStateOptions = {
  attention: AutoRouteAttention | null;
  configuredProviders: ConfiguredProvider[];
  dynamicRoutingEnabled: boolean;
  localModelId: string;
  resolveChoice: (choice: AutoRouteRecoveryAction) => void | Promise<void>;
  sessionId: string;
};

export function useAutoRouteRuntimeState({
  attention,
  configuredProviders,
  dynamicRoutingEnabled,
  localModelId,
  resolveChoice,
  sessionId,
}: AutoRouteRuntimeStateOptions) {
  const cloudModelId = useMemo(() => {
    const target = configuredProviders.find((provider) =>
      provider.autoRouteTarget && configuredProviderIsRunnable(provider));
    return target?.customModelIds
      .split(/[,\n]/)
      .map((modelId) => modelId.trim())
      .find(Boolean) ?? "";
  }, [configuredProviders]);
  const readiness = useAutoRouteReadiness({
    sessionId,
    dynamicRoutingEnabled,
    localModelId,
  });

  useEffect(() => {
    const readinessRestored = readiness.autoRouteSessionReadiness.status === "ready";
    const recoveryCanContinue = attention?.kind === "preparing"
      && attention.continueWhenReady;
    if (attention?.sessionId === sessionId && readinessRestored && recoveryCanContinue) {
      void resolveChoice("retry");
    }
  }, [attention, cloudModelId, readiness.autoRouteSessionReadiness.status, resolveChoice, sessionId]);

  return { ...readiness, autoRouteCloudModelId: cloudModelId };
}
