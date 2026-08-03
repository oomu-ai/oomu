"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@/lib/invoke";

export const RECOMMENDED_MODEL_INSTALL_EVENT = "recommended-model-install-progress";

export type RecommendedModelInstallPhase =
  | "idle"
  | "downloading"
  | "cancelled"
  | "verifying"
  | "inspecting"
  | "configuring"
  | "preparing"
  | "ready"
  | "failed";

export type RecommendedModelProviderEvidence = {
  providerId: string;
  providerType: "local_model";
  modelId: string;
  verified: boolean;
};

export type RecommendedModelInstallProgress = {
  installId: string | null;
  state: RecommendedModelInstallPhase;
  downloadedBytes: number;
  totalBytes: number;
  currentAsset: string | null;
  canCancel: boolean;
  canResume: boolean;
  publicErrorCode: string | null;
  completedProvider: RecommendedModelProviderEvidence | null;
  locationDisplayPath?: string | null;
  locationKind?: "managed" | "granted" | null;
  packageState?: string | null;
};

export type RecommendedModelLocationGrant = {
  locationGrantId: string;
  displayPath: string;
};

type RecommendedModelInstallStateResponse = {
  manifest?: {
    modelId?: string;
    displayName?: string;
    totalBytes?: number;
  };
  location?: {
    kind?: "managed" | "granted";
    displayPath?: string;
    locationGrantId?: string;
  };
  packageState?: string;
  activeInstall?: unknown;
  receipt?: {
    completedProvider?: RecommendedModelProviderEvidence | null;
    provider?: RecommendedModelProviderEvidence | null;
  } | null;
};

const EMPTY_PROGRESS: RecommendedModelInstallProgress = {
  installId: null,
  state: "idle",
  downloadedBytes: 0,
  totalBytes: 4_336_349_920,
  currentAsset: null,
  canCancel: false,
  canResume: false,
  publicErrorCode: null,
  completedProvider: null,
  locationDisplayPath: null,
  locationKind: null,
  packageState: null,
};

function finiteByteCount(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : fallback;
}

const INSTALL_PHASES: Record<string, RecommendedModelInstallPhase> = {
  downloading: "downloading",
  cancelled: "cancelled",
  verifying: "verifying",
  inspecting: "inspecting",
  configuring: "configuring",
  preparing: "preparing",
  ready: "ready",
  failed: "failed",
  partial: "cancelled",
  promoting: "preparing",
  repairRequired: "failed",
  repair_required: "failed",
  prewarming: "preparing",
  complete: "ready",
  completed: "ready",
};

function installPhase(value: unknown): RecommendedModelInstallPhase {
  return typeof value === "string" ? INSTALL_PHASES[value] ?? "idle" : "idle";
}

function installId(
  payload: Record<string, unknown>,
  previous: RecommendedModelInstallProgress,
  discarded: boolean,
) {
  if (discarded) return null;
  if (typeof payload.installId === "string") return payload.installId;
  if (typeof payload.install_id === "string") return payload.install_id;
  return previous.installId;
}

export function normalizeRecommendedModelInstallProgress(
  value: unknown,
  previous: RecommendedModelInstallProgress = EMPTY_PROGRESS,
): RecommendedModelInstallProgress {
  if (!value || typeof value !== "object" || Array.isArray(value)) return previous;
  const root = value as Record<string, unknown>;
  const payload = root.progress && typeof root.progress === "object"
    ? root.progress as Record<string, unknown>
    : root.activeInstall && typeof root.activeInstall === "object"
      ? root.activeInstall as Record<string, unknown>
      : root;
  const stateResponse = root as RecommendedModelInstallStateResponse;
  const location = stateResponse.location;
  const manifest = stateResponse.manifest;
  const receiptProvider = stateResponse.receipt?.provider
    ?? stateResponse.receipt?.completedProvider;
  const completedProvider = payload.completedProvider;
  const packageState = typeof stateResponse.packageState === "string"
    ? stateResponse.packageState
    : undefined;
  const normalizedState = installPhase(payload.state ?? payload.phase ?? packageState);
  const discarded = root.discarded === true;
  return {
    installId: installId(payload, previous, discarded),
    state:
      normalizedState === "idle" && receiptProvider?.verified
        ? "ready"
        : normalizedState,
    downloadedBytes: discarded
      ? 0
      : finiteByteCount(
          payload.downloadedBytes ?? payload.downloaded_bytes,
          previous.downloadedBytes,
        ),
    totalBytes: finiteByteCount(
      payload.totalBytes ?? payload.total_bytes ?? manifest?.totalBytes,
      previous.totalBytes,
    ),
    currentAsset:
      typeof (payload.currentAsset ?? payload.current_asset) === "string"
        ? String(payload.currentAsset ?? payload.current_asset)
        : null,
    canCancel: Boolean(payload.canCancel ?? payload.can_cancel),
    canResume: discarded ? false : Boolean(payload.canResume ?? payload.can_resume),
    publicErrorCode:
      typeof (payload.publicErrorCode ?? payload.public_error_code) === "string"
        ? String(payload.publicErrorCode ?? payload.public_error_code)
        : null,
    completedProvider:
      completedProvider && typeof completedProvider === "object"
        ? (completedProvider as RecommendedModelProviderEvidence)
        : receiptProvider ?? null,
    locationDisplayPath:
      typeof (payload.locationDisplayPath ?? payload.location_display_path ?? location?.displayPath) === "string"
        ? String(payload.locationDisplayPath ?? payload.location_display_path ?? location?.displayPath)
        : previous.locationDisplayPath,
    locationKind:
      location?.kind === "managed" || location?.kind === "granted"
        ? location.kind
        : previous.locationKind,
    packageState: packageState ?? previous.packageState,
  };
}

function publicFailureCode(cause: unknown) {
  if (cause && typeof cause === "object" && "code" in cause) {
    const code = String((cause as { code?: unknown }).code ?? "").trim();
    if (code) return code;
  }
  return "model_install_failed";
}

export function useRecommendedLocalModelInstall() {
  const [progress, setProgress] = useState<RecommendedModelInstallProgress>(EMPTY_PROGRESS);
  const [locationGrant, setLocationGrant] = useState<RecommendedModelLocationGrant | null>(null);
  const [loading, setLoading] = useState(true);
  const operationRef = useRef<Promise<void> | null>(null);

  const applyProgress = useCallback((value: unknown) => {
    setProgress((current) => normalizeRecommendedModelInstallProgress(value, current));
  }, []);

  const refresh = useCallback(async () => {
    const next = await invoke<unknown>("get_recommended_model_install_state");
    applyProgress(next);
  }, [applyProgress]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void import("@tauri-apps/api/event")
      .then(async ({ listen }) => {
        const stopListening = await listen<unknown>(RECOMMENDED_MODEL_INSTALL_EVENT, (event) => {
          if (active) applyProgress(event.payload);
        });
        if (!active) {
          stopListening();
          return;
        }
        unlisten = stopListening;
        if (active) await refresh();
      })
      .catch((cause) => {
        if (active) {
          setProgress((current) => ({
            ...current,
            state: "failed",
            publicErrorCode: publicFailureCode(cause),
          }));
        }
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [applyProgress, refresh]);

  const runSingleFlight = useCallback((operation: () => Promise<unknown>) => {
    if (operationRef.current) return operationRef.current;
    const pending = operation()
      .then(applyProgress)
      .catch((cause) => {
        setProgress((current) => ({
          ...current,
          state: "failed",
          canCancel: false,
          publicErrorCode: publicFailureCode(cause),
        }));
      })
      .finally(() => {
        operationRef.current = null;
      });
    operationRef.current = pending;
    return pending;
  }, [applyProgress]);

  const chooseLocation = useCallback(async (dialogTitle: string) => {
    try {
      const grant = await invoke<RecommendedModelLocationGrant | null>(
        "choose_recommended_model_install_location",
        { dialogTitle },
      );
      if (grant?.locationGrantId && grant.displayPath) setLocationGrant(grant);
    } catch (cause) {
      setProgress((current) => ({
        ...current,
        state: "failed",
        publicErrorCode: publicFailureCode(cause),
      }));
    }
  }, []);

  const start = useCallback(() => runSingleFlight(() =>
    invoke<unknown>("start_recommended_model_install", {
      locationGrantId: locationGrant?.locationGrantId ?? null,
    })), [locationGrant, runSingleFlight]);

  const cancel = useCallback(() => {
    if (!progress.installId) return Promise.resolve();
    return runSingleFlight(() => invoke<unknown>("cancel_recommended_model_install", {
      installId: progress.installId,
    }));
  }, [progress.installId, runSingleFlight]);

  const discard = useCallback(() => {
    if (!progress.installId) return Promise.resolve();
    return runSingleFlight(() => invoke<unknown>("discard_recommended_model_partial", {
      installId: progress.installId,
    }));
  }, [progress.installId, runSingleFlight]);

  return {
    progress,
    locationGrant,
    loading,
    chooseLocation,
    start,
    cancel,
    discard,
    refresh,
  };
}
