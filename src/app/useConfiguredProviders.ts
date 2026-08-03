import { invoke } from "@/lib/invoke";
import { parseModelIds, type ConfiguredProvider } from "@/lib/modelRegistry";
import { useEffect, useState } from "react";
import type { LocalModelCompatibility } from "./homeAgents";

export function useConfiguredProviders(licenseAccepted: boolean | undefined) {
  const [configuredProviders, setConfiguredProviders] = useState<ConfiguredProvider[]>([]);

  useEffect(() => {
    if (!licenseAccepted) return;
    let cancelled = false;

    async function loadProviderConfigs() {
      try {
        const configs = await invoke<ConfiguredProvider[]>("list_provider_configs");
        if (!cancelled) setConfiguredProviders(configs);
        const hasLocalProvider = configs.some(
          (provider) => provider.providerId === "local_model",
        );
        if (!hasLocalProvider) return;

        try {
          const localModels = await invoke<LocalModelCompatibility[]>("list_local_models");
          const readyLocalModelIds = new Set(
            localModels
              .filter((model) => model.compatibility === "ready")
              .map((model) => model.id),
          );
          if (readyLocalModelIds.size === 0) return;

          const reconciledConfigs = await Promise.all(
            configs.map(async (provider) => {
              if (provider.providerId !== "local_model") return provider;

              const runnableModelIds = parseModelIds(provider.customModelIds).filter(
                (modelId) => readyLocalModelIds.has(modelId),
              );
              const fallbackModelId = localModels.find(
                (model) => model.compatibility === "ready",
              )?.id;
              const nextModelIds =
                runnableModelIds.length > 0
                  ? runnableModelIds
                  : fallbackModelId
                    ? [fallbackModelId]
                    : [];
              const customModelIds = nextModelIds.join("\n");

              if (customModelIds === provider.customModelIds) return provider;

              return invoke<ConfiguredProvider>("save_provider_config", {
                request: { ...provider, customModelIds },
              });
            }),
          );

          if (!cancelled) setConfiguredProviders(reconciledConfigs);
        } catch (error) {
          console.error("Failed to reconcile local provider models:", error);
        }
      } catch (error) {
        console.error("Failed to load provider configs:", error);
      }
    }

    void loadProviderConfigs();
    return () => {
      cancelled = true;
    };
  }, [licenseAccepted]);

  return [configuredProviders, setConfiguredProviders] as const;
}
