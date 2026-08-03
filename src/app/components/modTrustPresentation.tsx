"use client";

import { useI18n } from "@/context/I18nContext";

export type ModReviewState = "reviewed" | "unreviewed" | "revoked";
export type ModIntegrityState = "verified" | "unsigned" | "modified" | "unknown";

type ModTrustRecord = {
  reviewState?: ModReviewState;
  publisherIdentityVerified?: boolean;
  integrityState?: ModIntegrityState;
};

type TrustPresentation = {
  tone: "reviewed" | "neutral" | "warm";
  labelKey: string;
  detailKey: string;
};

export function modTrustPresentation({
  integrityState,
  publisherIdentityVerified,
  reviewState,
}: ModTrustRecord): TrustPresentation {
  if (reviewState === "revoked") {
    return { tone: "warm", labelKey: "mods.review_withdrawn", detailKey: "mods.review_withdrawn_explanation" };
  }
  if (integrityState === "modified") {
    return { tone: "warm", labelKey: "mods.modified_mod", detailKey: "mods.modified_mod_explanation" };
  }
  if (reviewState === "reviewed" && integrityState === "verified") {
    return { tone: "reviewed", labelKey: "mods.reviewed_by_oomu", detailKey: "mods.reviewed_explanation" };
  }
  if (reviewState === "unreviewed" && publisherIdentityVerified === true) {
    return { tone: "neutral", labelKey: "mods.custom_mod", detailKey: "mods.custom_mod_explanation" };
  }
  if (reviewState === "unreviewed") {
    return { tone: "neutral", labelKey: "mods.not_reviewed", detailKey: "mods.not_reviewed_explanation" };
  }
  return { tone: "neutral", labelKey: "mods.review_unknown", detailKey: "mods.review_unknown_explanation" };
}

const BADGE_TONES: Record<TrustPresentation["tone"], string> = {
  reviewed: "border-[var(--success)]/30 bg-[var(--success-background)] text-[var(--success)]",
  neutral: "border-[var(--border-soft)] bg-[var(--accent-background)] text-[var(--foreground-muted)]",
  warm: "border-[var(--warning)]/35 bg-[var(--warning-background)] text-[var(--foreground)]",
};

export function ModTrustBadge({ presentation }: { presentation: TrustPresentation }) {
  const { t } = useI18n();
  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold ${BADGE_TONES[presentation.tone]}`}>
      {t(presentation.labelKey)}
    </span>
  );
}

export function ModTrustSummary({ presentation }: { presentation: TrustPresentation }) {
  const { t } = useI18n();
  const tone = presentation.tone === "warm"
    ? "border-[var(--warning)]/35 bg-[var(--warning-background)]"
    : presentation.tone === "reviewed"
      ? "border-[var(--success)]/30 bg-[var(--success-background)]"
      : "border-[var(--border-soft)] bg-[var(--accent-background)]";
  return (
    <div className={`rounded-[var(--radius-sm)] border p-3 ${tone}`}>
      <ModTrustBadge presentation={presentation} />
      <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">{t(presentation.detailKey)}</p>
    </div>
  );
}
