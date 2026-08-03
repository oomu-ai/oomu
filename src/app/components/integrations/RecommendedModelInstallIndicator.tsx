"use client";

import { useI18n } from "@/context/I18nContext";
import { requestRecommendedModelSettings } from "./recommendedModelSettingsRoute";
import {
  useRecommendedLocalModelInstall,
  type RecommendedModelInstallProgress,
} from "./useRecommendedLocalModelInstall";

const VISIBLE_PHASES = new Set([
  "downloading",
  "verifying",
  "inspecting",
  "configuring",
  "preparing",
]);

function percentage(progress: RecommendedModelInstallProgress) {
  if (progress.totalBytes <= 0) return 0;
  return Math.min(100, Math.max(0, Math.floor(
    progress.downloadedBytes / progress.totalBytes * 100,
  )));
}

function statusLabel(progress: RecommendedModelInstallProgress, t: (key: string) => string) {
  if (progress.state === "downloading") {
    return `${t("recommended_model.downloading")} ${percentage(progress)}%`;
  }
  if (progress.state === "verifying" || progress.state === "inspecting") {
    return t("recommended_model.verifying");
  }
  return t("recommended_model.preparing");
}

export function RecommendedModelInstallIndicator({
  onOpenModels,
}: {
  onOpenModels: () => void;
}) {
  const { language, t } = useI18n();
  const { progress } = useRecommendedLocalModelInstall();
  if (!VISIBLE_PHASES.has(progress.state)) return null;

  const percent = percentage(progress);
  const label = statusLabel(progress, t);
  const exactProgress = t("recommended_model.download_progress", {
    downloaded: new Intl.NumberFormat(language, { maximumFractionDigits: 2 }).format(
      progress.downloadedBytes / 1_000_000_000,
    ),
    total: new Intl.NumberFormat(language, { maximumFractionDigits: 2 }).format(
      progress.totalBytes / 1_000_000_000,
    ),
  });
  const openModels = () => {
    requestRecommendedModelSettings();
    onOpenModels();
  };

  return (
    <button
      aria-label={`${label}. ${t("settings.tabs.models")}`}
      className="group flex select-none items-center gap-2 rounded-full border border-[var(--border-soft)] bg-[var(--background)] px-3 py-1.5 text-[11px] font-semibold text-[var(--foreground)] shadow-sm transition-colors hover:bg-[var(--fill-hover)]"
      data-recommended-model-install-indicator
      onClick={openModels}
      title={progress.state === "downloading" ? exactProgress : label}
      type="button"
    >
      {progress.state === "downloading" ? (
        <span
          aria-label={t("recommended_model.download_progress_label")}
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          aria-valuetext={exactProgress}
          className="h-1.5 w-14 overflow-hidden rounded-full bg-[var(--border-soft)]"
          role="progressbar"
        >
          <span
            className="block h-full rounded-full bg-[var(--accent)] transition-[width] motion-reduce:transition-none"
            style={{ width: `${percent}%` }}
          />
        </span>
      ) : (
        <span aria-hidden="true" className="h-2 w-2 animate-pulse rounded-full bg-[var(--accent)] motion-reduce:animate-none" />
      )}
      <span>{label}</span>
    </button>
  );
}
