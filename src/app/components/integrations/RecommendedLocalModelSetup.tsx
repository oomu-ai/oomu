"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { DEFAULT_LOCAL_MODEL_ID } from "@/lib/modelRegistry";
import {
  useRecommendedLocalModelInstall,
  type RecommendedModelInstallProgress,
  type RecommendedModelProviderEvidence,
} from "./useRecommendedLocalModelInstall";

type Translate = (key: string, variables?: Record<string, string | number>) => string;

type RecommendedLocalModelSetupProps = {
  disabled?: boolean;
  hasExistingReadyModel?: boolean;
  hideWhenReady?: boolean;
  onChooseExisting?: () => void | Promise<void>;
  onDefer?: () => void | Promise<void>;
  onUseExisting?: () => void | Promise<void>;
  onVerified?: (provider: RecommendedModelProviderEvidence) => void | Promise<void>;
};

function localizedInstallErrorKey(code: string | null) {
  if (!code) return "generic";
  if (code.endsWith("cancelled")) return "cancelled";
  if (/(storage|insufficient)/.test(code)) return "storage";
  if (/(location|destination|path|symlink|application_bundle|staging|collision|grant)/.test(code)) {
    return "destination";
  }
  if (/(integrity|hash|size_mismatch|content_range|validator|manifest|inspection)/.test(code)) {
    return "verification";
  }
  if (/(download|transport|dns|http|timeout|redirect|asset|headers|network)/.test(code)) {
    return "download";
  }
  if (/(prewarm|provider|setup|promotion|configur)/.test(code)) return "preparation";
  return "generic";
}

function formatDownloadBytes(bytes: number, language: string) {
  return new Intl.NumberFormat(language, {
    maximumFractionDigits: 2,
    minimumFractionDigits: 2,
  }).format(bytes / 1_000_000_000);
}

function phaseLabel(progress: RecommendedModelInstallProgress, t: Translate) {
  switch (progress.state) {
    case "downloading": return t("recommended_model.downloading");
    case "verifying":
    case "inspecting": return t("recommended_model.verifying");
    case "configuring":
    case "preparing": return t("recommended_model.preparing");
    case "ready": return t("recommended_model.ready");
    case "cancelled": return t("recommended_model.paused");
    case "failed": return t("recommended_model.needs_attention");
    default: return "";
  }
}

function ModelIdentity({ phase, t }: { phase: string; t: Translate }) {
  return (
    <div className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <p className="text-xs font-semibold text-[var(--foreground-muted)]">
          {t("recommended_model.eyebrow")}
        </p>
        <h2 className="mt-1 text-lg font-semibold" id="recommended-model-title">
          {t("recommended_model.full_name")}
        </h2>
        <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
          {t("recommended_model.optimized_for")}
        </p>
        <p className="mt-1 text-sm font-medium text-[var(--foreground)]">
          {t("recommended_model.local_download_size")}
        </p>
      </div>
      {phase ? <p className="shrink-0 text-sm font-semibold" role="status">{phase}</p> : null}
    </div>
  );
}

function InstallLocation({
  active, chooseLocation, disabled, locationKind, locationPath, t,
}: {
  active: boolean;
  chooseLocation: (dialogTitle: string) => Promise<void>;
  disabled: boolean;
  locationKind?: "managed" | "granted" | null;
  locationPath?: string | null;
  t: Translate;
}) {
  return (
    <div className="mt-5 rounded-[var(--radius-sm)] bg-[var(--fill-secondary)] p-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-xs font-semibold text-[var(--foreground-muted)]">
            {t("recommended_model.location")}
          </p>
          <p className="mt-1 truncate text-sm" title={locationPath || undefined}>
            {locationKind === "granted" && locationPath
              ? t("recommended_model.custom_location", { path: locationPath })
              : t("recommended_model.managed_location")}
          </p>
        </div>
        <button
          className="rounded px-2 py-1 text-sm font-semibold hover:bg-[var(--fill-hover)] disabled:opacity-50"
          disabled={disabled || active}
          onClick={() => void chooseLocation(t("recommended_model.change_location"))}
          type="button"
        >
          {t("recommended_model.change_location")}
        </button>
      </div>
    </div>
  );
}

function DownloadFeedback({
  factualProgress, percent, progress, t,
}: {
  factualProgress: string;
  percent: number;
  progress: RecommendedModelInstallProgress;
  t: Translate;
}) {
  if (progress.state === "failed") {
    return (
      <p className="mt-4 text-sm leading-6 text-[var(--warning)]" role="alert">
        {t(`recommended_model.errors.${localizedInstallErrorKey(progress.publicErrorCode)}`)}
      </p>
    );
  }
  if (progress.state !== "downloading") return null;
  return (
    <div className="mt-5">
      <p className="text-sm text-[var(--foreground-muted)]">{factualProgress}</p>
      <div
        aria-label={t("recommended_model.download_progress_label")}
        aria-valuemax={progress.totalBytes}
        aria-valuemin={0}
        aria-valuenow={progress.downloadedBytes}
        aria-valuetext={factualProgress}
        className="mt-2 h-2 overflow-hidden rounded-full bg-[var(--border-soft)]"
        role="progressbar"
      >
        <div
          className="h-full rounded-full bg-[var(--accent)] transition-[width] motion-reduce:transition-none"
          style={{ width: `${percent}%` }}
        />
      </div>
    </div>
  );
}

type InstallActionsProps = RecommendedLocalModelSetupProps & {
  active: boolean;
  cancel: () => Promise<void>;
  discardable: boolean;
  discard: () => Promise<void>;
  loading: boolean;
  packageAvailable: boolean;
  progress: RecommendedModelInstallProgress;
  retryVerified: () => Promise<void>;
  resumable: boolean;
  start: () => Promise<void>;
  t: Translate;
  verified: boolean;
  verifiedDeliveryFailed: boolean;
};

function InstallActions({
  active, cancel, disabled = false, discardable, discard, hasExistingReadyModel, loading,
  onChooseExisting, onDefer, onUseExisting, packageAvailable, progress,
  retryVerified, resumable, start, t, verified, verifiedDeliveryFailed,
}: InstallActionsProps) {
  return (
    <>
      <div className="mt-5 flex flex-wrap items-center gap-3">
        {!active && (!verified || verifiedDeliveryFailed) ? (
          <button
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2.5 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
            disabled={disabled || loading}
            onClick={() => void (verified ? retryVerified() : start())}
            type="button"
          >
            {verified ? t("recommended_model.use_and_continue") : resumable
              ? t("recommended_model.resume") : packageAvailable
              ? t("recommended_model.use_and_continue")
              : t("recommended_model.download_and_continue")}
          </button>
        ) : null}
        {progress.canCancel ? (
          <button className="rounded-[var(--radius-sm)] border px-4 py-2 text-sm font-semibold disabled:opacity-50" disabled={disabled} onClick={() => void cancel()} type="button">
            {t("common.cancel")}
          </button>
        ) : null}
        {discardable && progress.installId ? (
          <button className="rounded px-2 py-1 text-sm font-semibold text-[var(--foreground-muted)] hover:text-[var(--foreground)] disabled:opacity-50" disabled={disabled || active} onClick={() => void discard()} type="button">
            {t("recommended_model.remove_partial")}
          </button>
        ) : null}
        {hasExistingReadyModel && onUseExisting ? (
          <button className="rounded-[var(--radius-sm)] border px-4 py-2 text-sm font-semibold disabled:opacity-50" disabled={disabled || active} onClick={() => void onUseExisting()} type="button">
            {t("recommended_model.use_existing_and_continue")}
          </button>
        ) : null}
      </div>
      <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-[var(--border-soft)] pt-4">
        {onChooseExisting ? (
          <button className="text-sm font-semibold text-[var(--foreground-muted)] hover:text-[var(--foreground)] disabled:opacity-50" disabled={disabled || active} onClick={() => void onChooseExisting()} type="button">
            {t("recommended_model.choose_existing")}
          </button>
        ) : null}
        {onDefer ? (
          <button className="text-sm font-semibold text-[var(--foreground-muted)] hover:text-[var(--foreground)]" onClick={() => void onDefer()} type="button">
            {t("recommended_model.set_up_later")}
          </button>
        ) : null}
        <details className="ml-auto text-sm text-[var(--foreground-muted)]">
          <summary className="cursor-pointer font-semibold">{t("recommended_model.about_build")}</summary>
          <p className="mt-2 max-w-xl leading-6">{t("recommended_model.about_build_help")}</p>
        </details>
      </div>
    </>
  );
}

export function RecommendedLocalModelSetup(props: RecommendedLocalModelSetupProps) {
  const { disabled = false, hideWhenReady = false, onVerified } = props;
  const { language, t } = useI18n();
  const install = useRecommendedLocalModelInstall();
  const { progress } = install;
  const deliveredReceiptRef = useRef("");
  const deliveringReceiptRef = useRef("");
  const onVerifiedRef = useRef(onVerified);
  const [deliveryState, setDeliveryState] = useState<"idle" | "delivering" | "delivered" | "failed">("idle");
  const provider = progress.completedProvider;
  const verified = Boolean(provider?.verified && provider.providerType === "local_model"
    && provider.modelId === DEFAULT_LOCAL_MODEL_ID);

  useEffect(() => {
    onVerifiedRef.current = onVerified;
  }, [onVerified]);

  const deliverVerified = useCallback(async () => {
    const callback = onVerifiedRef.current;
    if (!verified || !provider || !callback) return;
    const key = `${progress.installId ?? "installed"}:${provider.providerId}:${provider.modelId}`;
    if (deliveredReceiptRef.current === key || deliveringReceiptRef.current === key) return;
    deliveringReceiptRef.current = key;
    setDeliveryState("delivering");
    try {
      await callback(provider);
      deliveredReceiptRef.current = key;
      setDeliveryState("delivered");
    } catch {
      if (deliveredReceiptRef.current === key) deliveredReceiptRef.current = "";
      setDeliveryState("failed");
    } finally {
      if (deliveringReceiptRef.current === key) deliveringReceiptRef.current = "";
    }
  }, [progress.installId, provider, verified]);

  useEffect(() => {
    const timer = window.setTimeout(() => void deliverVerified(), 0);
    return () => window.clearTimeout(timer);
  }, [deliverVerified]);

  const percent = progress.totalBytes > 0
    ? Math.min(100, Math.max(0, (progress.downloadedBytes / progress.totalBytes) * 100)) : 0;
  const factualProgress = t("recommended_model.download_progress", {
    downloaded: formatDownloadBytes(progress.downloadedBytes, language),
    total: formatDownloadBytes(progress.totalBytes, language),
  });
  const nativePhase = useMemo(() => phaseLabel(progress, t), [progress, t]);
  const phase = deliveryState === "failed"
    ? t("recommended_model.needs_attention")
    : verified && onVerified && deliveryState !== "delivered"
      ? t("recommended_model.preparing")
      : nativePhase;
  const active = ["downloading", "verifying", "inspecting", "configuring", "preparing"]
    .includes(progress.state);
  const announcement = progress.state === "downloading"
    ? t("recommended_model.progress_announcement", { percent: Math.floor(percent / 10) * 10 })
    : phase;
  const resumable = progress.canResume || progress.state === "cancelled";
  const discardable = progress.state === "cancelled" || progress.state === "failed";
  const packageAvailable = ["verified", "installed", "adoptable", "ready"]
    .includes(progress.packageState ?? "");
  if (hideWhenReady && verified && (!onVerified || deliveryState === "delivered")) return null;

  return (
    <section aria-labelledby="recommended-model-title" className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5" data-recommended-model-setup>
      <p aria-live="polite" className="sr-only">{announcement}</p>
      <ModelIdentity phase={phase} t={t} />
      <InstallLocation active={active} chooseLocation={install.chooseLocation} disabled={disabled} locationKind={install.locationGrant ? "granted" : progress.locationKind} locationPath={install.locationGrant?.displayPath || progress.locationDisplayPath} t={t} />
      <DownloadFeedback factualProgress={factualProgress} percent={percent} progress={progress} t={t} />
      <InstallActions {...props} active={active} cancel={install.cancel} discardable={discardable} discard={install.discard} loading={install.loading} packageAvailable={packageAvailable} progress={progress} retryVerified={deliverVerified} resumable={resumable} start={install.start} t={t} verified={verified} verifiedDeliveryFailed={deliveryState === "failed"} />
    </section>
  );
}
