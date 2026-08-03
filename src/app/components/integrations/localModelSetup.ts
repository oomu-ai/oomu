import { invoke } from "@/lib/invoke";
import {
  DEFAULT_LOCAL_MODEL_ID,
  parseModelIds,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";

type LocalSetupModel = {
  id: string;
  compatibility: string;
};

type LocalSetupSelection = {
  localModels: LocalSetupModel[];
  providerConfigs: ConfiguredProvider[];
  providerName: string;
  requiredModelId?: string;
  onProviderConfigured?: (provider: ConfiguredProvider) => void;
};

export type LocalSetupEvidence = {
  providers: ConfiguredProvider[];
  savedProvider: ConfiguredProvider;
  selectedModelId: string;
};

export function verifiedRecommendedProvider(
  provider: ConfiguredProvider | null | undefined,
): provider is ConfiguredProvider {
  return Boolean(
    provider &&
      provider.providerId === "local_model" &&
      parseModelIds(provider.customModelIds).includes(DEFAULT_LOCAL_MODEL_ID),
  );
}

export async function persistLocalSetupSelection({
  localModels,
  providerConfigs,
  providerName,
  requiredModelId,
  onProviderConfigured,
}: LocalSetupSelection): Promise<LocalSetupEvidence> {
  const readyModels = localModels.filter((model) => model.compatibility === "ready");
  if (readyModels.length === 0) throw { code: "setup_model_execution_failed" };

  const preferred = requiredModelId
    ? { modelId: requiredModelId }
    : await invoke<{ modelId: string }>("get_default_prewarmed_model");
  const selectedModel = readyModels.find((model) => model.id === preferred.modelId)
    ?? (requiredModelId ? undefined : readyModels[0]);
  if (!selectedModel) throw { code: "setup_model_execution_failed" };
  await invoke("set_default_prewarmed_model", {
    modelId: selectedModel.id,
    model_id: selectedModel.id,
  });

  const existing = providerConfigs.find(
    (provider) => provider.providerId === "local_model",
  );
  const saved = await invoke<ConfiguredProvider>("save_provider_config", {
    request: {
      id: existing?.id ?? "local-model",
      providerId: "local_model",
      providerName: existing?.providerName || providerName,
      authMethod: "custom",
      baseUrl: "",
      apiKeyLabel: "",
      customModelIds: [
        selectedModel.id,
        ...readyModels.map((model) => model.id).filter((id) => id !== selectedModel.id),
      ].join("\n"),
      autoRouteTarget: false,
      createdAtMs: existing?.createdAtMs ?? 0,
      updatedAtMs: existing?.updatedAtMs ?? 0,
    },
  });
  onProviderConfigured?.(saved);
  return {
    providers: [saved, ...providerConfigs.filter((provider) => provider.id !== saved.id)],
    savedProvider: saved,
    selectedModelId: selectedModel.id,
  };
}
