"use client";

import type { ReactNode } from "react";
import { useI18n } from "@/context/I18nContext";

export function DocumentReviewShell({
  actions,
  details,
  detailsId,
  detailsOpen,
  kind,
  preview,
  revision,
  status,
  title,
  warnings,
  onDetailsToggle,
}: {
  actions: ReactNode;
  details: ReactNode;
  detailsId?: string;
  detailsOpen?: boolean;
  kind: string;
  preview: ReactNode;
  revision: number;
  status: ReactNode;
  title: string;
  warnings?: ReactNode;
  onDetailsToggle?: (open: boolean) => void;
}) {
  const { t } = useI18n();
  return (
    <div>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="text-xs font-medium text-[var(--foreground-muted)]">{kind}</p>
          <h2 className="mt-1 text-xl font-semibold">{title}</h2>
          <p className="mt-1 text-xs text-[var(--foreground-muted)]">
            {t("documents.revision", { revision })}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">{actions}</div>
      </div>
      <div className="mt-5">{preview}</div>
      <div className="mt-4 rounded bg-[var(--accent-background)] p-3 text-sm" role="status">
        {status}
      </div>
      {warnings ? <div className="mt-5">{warnings}</div> : null}
      <details className="mt-5 rounded border border-[var(--border-soft)] p-4" id={detailsId} onToggle={(event) => onDetailsToggle?.(event.currentTarget.open)} open={detailsOpen}>
        <summary className="cursor-pointer text-sm font-semibold">{t("documents.details")}</summary>
        <div className="mt-4">{details}</div>
      </details>
    </div>
  );
}
