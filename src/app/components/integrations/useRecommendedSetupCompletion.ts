"use client";

import { useRef } from "react";
import { invoke } from "@/lib/invoke";
import {
  DEFAULT_LOCAL_MODEL_ID,
  type ConfiguredProvider,
} from "@/lib/modelRegistry";
import { verifiedRecommendedProvider } from "./localModelSetup";
import type { RecommendedModelProviderEvidence } from "./useRecommendedLocalModelInstall";

type LocalModel = {
  id: string;
  compatibility: string;
};

type RecommendedSetupCompletionOptions = {
  advance: () => Promise<void>;
  applyProviders: (providers: ConfiguredProvider[]) => void;
  onError: (cause: unknown) => void;
  onProviderConfigured?: (provider: ConfiguredProvider) => void;
  refreshLocalModels: () => Promise<LocalModel[]>;
  setBusy: (busy: boolean) => void;
};

export function useRecommendedSetupCompletion({
  advance,
  applyProviders,
  onError,
  onProviderConfigured,
  refreshLocalModels,
  setBusy,
}: RecommendedSetupCompletionOptions) {
  const advancedReceiptRef = useRef("");
  const deferredRef = useRef(false);
  const deferInFlightRef = useRef(false);

  async function defer() {
    if (deferInFlightRef.current) return;
    deferInFlightRef.current = true;
    deferredRef.current = true;
    setBusy(true);
    try {
      await advance();
    } catch (cause) {
      deferredRef.current = false;
      onError(cause);
    } finally {
      deferInFlightRef.current = false;
      setBusy(false);
    }
  }

  async function accept(evidence: RecommendedModelProviderEvidence) {
    const receiptKey = `${evidence.providerId}:${evidence.modelId}`;
    if (advancedReceiptRef.current === receiptKey) return;
    setBusy(true);
    try {
      const [models, providers] = await Promise.all([
        refreshLocalModels(),
        invoke<ConfiguredProvider[]>("list_provider_configs"),
      ]);
      const exactModelReady = models.some(
        (model) => model.id === DEFAULT_LOCAL_MODEL_ID && model.compatibility === "ready",
      );
      const savedProvider = providers.find((provider) => provider.id === evidence.providerId);
      if (!exactModelReady || !verifiedRecommendedProvider(savedProvider)) {
        throw { code: "setup_model_execution_failed" };
      }
      applyProviders(providers);
      onProviderConfigured?.(savedProvider);
      if (deferredRef.current) {
        advancedReceiptRef.current = receiptKey;
        return;
      }
      await advance();
      advancedReceiptRef.current = receiptKey;
    } catch (cause) {
      advancedReceiptRef.current = "";
      onError(cause);
      throw cause;
    } finally {
      setBusy(false);
    }
  }

  return { accept, defer };
}
