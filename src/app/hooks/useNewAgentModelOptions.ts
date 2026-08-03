"use client";

import {
  configuredModelOptions,
  modelsForProvider,
  providerOptionsFromConfigured,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import { useMemo } from "react";
import { defaultLocalAgentEndpoint, type AgentModelProvider } from "../homeAgents";

export function useNewAgentModelOptions(
  configuredProviders: ConfiguredProvider[],
  requestedProvider: AgentModelProvider,
  requestedModelId: string,
  verifiedStartupModelId: string | null,
) {
  const providerOptions = useMemo(
    () => providerOptionsFromConfigured(configuredProviders),
    [configuredProviders],
  );
  const configuredModels = useMemo(
    () => configuredModelOptions(configuredProviders),
    [configuredProviders],
  );
  const provider = configuredModels.some((model) => model.providerId === requestedProvider)
    ? requestedProvider
    : configuredModels[0]?.providerId ?? requestedProvider;
  const models = useMemo(
    () => modelsForProvider(configuredProviders, provider),
    [configuredProviders, provider],
  );
  const modelId = models.some((model) => model.modelId === requestedModelId)
    ? requestedModelId
    : provider === defaultLocalAgentEndpoint.provider
      ? verifiedStartupModelId ?? ""
      : "";
  return {
    configuredModels,
    model: models.find((item) => item.modelId === modelId),
    modelId,
    models,
    provider,
    providerOptions,
  };
}
