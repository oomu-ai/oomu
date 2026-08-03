"use client";

import { useCallback, useEffect, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import {
  DEFAULT_LOCAL_MODEL_ID,
  parseModelIds,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import { persistLocalSetupSelection } from "./localModelSetup";
import { RecommendedLocalModelSetup } from "./RecommendedLocalModelSetup";

type LocalModel = {
  id: string;
  compatibility: string;
};

type RecommendedModelSettingsSetupProps = {
  configuredProviders: ConfiguredProvider[];
  onProvidersChange: (providers: ConfiguredProvider[]) => void;
};

export function RecommendedModelSettingsSetup({
  configuredProviders,
  onProvidersChange,
}: RecommendedModelSettingsSetupProps) {
  const { t } = useI18n();
  const [error, setError] = useState("");
  const [exactModelReady, setExactModelReady] = useState(false);
  const configured = configuredProviders.some(
    (provider) =>
      provider.providerId === "local_model" &&
      parseModelIds(provider.customModelIds).includes(DEFAULT_LOCAL_MODEL_ID),
  );

  const inspectLocalInventory = useCallback(async () => {
    try {
      const localModels = await invoke<LocalModel[]>("list_local_models");
      const ready = localModels.some(
        (model) => model.id === DEFAULT_LOCAL_MODEL_ID && model.compatibility === "ready",
      );
      setExactModelReady(ready);
      return ready;
    } catch (cause) {
      setExactModelReady(false);
      throw cause;
    }
  }, []);

  useEffect(() => {
    let active = true;
    void invoke<LocalModel[]>("list_local_models")
      .then((localModels) => {
        if (active) {
          setExactModelReady(localModels.some(
            (model) => model.id === DEFAULT_LOCAL_MODEL_ID && model.compatibility === "ready",
          ));
        }
      })
      .catch(() => {
        if (active) {
          setExactModelReady(false);
          setError(t("models.status.local_inspect_failed"));
        }
      });
    return () => { active = false; };
  }, [t]);

  async function refreshProviders() {
    setError("");
    try {
      const [providers, modelReady] = await Promise.all([
        invoke<ConfiguredProvider[]>("list_provider_configs"),
        inspectLocalInventory(),
      ]);
      const exactProviderReady = providers.some(
        (provider) => provider.providerId === "local_model"
          && parseModelIds(provider.customModelIds).includes(DEFAULT_LOCAL_MODEL_ID),
      );
      if (!exactProviderReady || !modelReady) {
        throw new Error("recommended model health was not confirmed");
      }
      onProvidersChange(providers);
    } catch {
      setError(t("models.status.local_inspect_failed"));
      throw new Error("recommended model health could not be confirmed");
    }
  }

  async function chooseExistingFolder() {
    setError("");
    try {
      const setting = await invoke<{ path: string; isDefault: boolean } | null>(
        "choose_local_model_directory",
      );
      if (!setting) return;
      const [localModels, providers] = await Promise.all([
        invoke<LocalModel[]>("list_local_models"),
        invoke<ConfiguredProvider[]>("list_provider_configs"),
      ]);
      const evidence = await persistLocalSetupSelection({
        localModels,
        providerConfigs: providers,
        providerName: t("models.provider_names.local_model"),
      });
      setExactModelReady(localModels.some(
        (model) => model.id === DEFAULT_LOCAL_MODEL_ID && model.compatibility === "ready",
      ));
      onProvidersChange(evidence.providers);
    } catch {
      setError(t("models.status.local_inspect_failed"));
    }
  }

  if (configured && exactModelReady) return null;
  return (
    <div className="mb-7 max-w-3xl shrink-0">
      <RecommendedLocalModelSetup
        hideWhenReady
        onChooseExisting={chooseExistingFolder}
        onVerified={refreshProviders}
      />
      {error ? (
        <p className="mt-3 text-sm text-[var(--warning)]" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
