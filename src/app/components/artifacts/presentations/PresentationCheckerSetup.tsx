"use client";

import { useCallback, useEffect, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import {
  presentationApi,
  type PresentationCheckerReadiness,
} from "./presentationClient";

export function PresentationCheckerSetup() {
  const { t } = useI18n();
  const [readiness, setReadiness] =
    useState<PresentationCheckerReadiness | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      setReadiness(await presentationApi.checkerReadiness());
    } catch {
      setReadiness(null);
      setError("probe_failed");
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    presentationApi
      .checkerReadiness()
      .then((next) => {
        if (active) setReadiness(next);
      })
      .catch(() => {
        if (active) setError("probe_failed");
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
    };
  }, []);

  const openDownload = async () => {
    setError("");
    try {
      await presentationApi.openCheckerDownload();
    } catch {
      setError("download_failed");
    }
  };

  const status = readiness?.status ?? "loading";
  const canOpenDownload =
    readiness?.status === "not_installed" ||
    readiness?.status === "not_qualified";

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="max-w-2xl">
          <h2 className="text-sm font-semibold text-[var(--foreground)]">
            {t("settings.general.presentation_checker.title")}
          </h2>
          <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
            {t("settings.general.presentation_checker.description")}
          </p>
          <div className="mt-4" role="status">
            <p className="text-sm font-semibold text-[var(--foreground)]">
              {t(`settings.general.presentation_checker.status.${status}`)}
            </p>
            <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
              {t(`settings.general.presentation_checker.detail.${status}`, {
                version: readiness?.requiredVersion ?? "",
              })}
            </p>
          </div>
          {error ? (
            <p className="mt-3 text-sm text-[var(--warning)]" role="alert">
              {t(`settings.general.presentation_checker.errors.${error}`)}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {canOpenDownload ? (
            <button
              className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)]"
              onClick={() => void openDownload()}
              type="button"
            >
              {t("settings.general.presentation_checker.open_download", {
                version: readiness.requiredVersion,
              })}
            </button>
          ) : null}
          <button
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-4 py-2 text-sm font-semibold disabled:opacity-50"
            disabled={busy}
            onClick={() => void refresh()}
            type="button"
          >
            {busy
              ? t("settings.general.presentation_checker.checking")
              : t("settings.general.presentation_checker.check_again")}
          </button>
        </div>
      </div>
    </section>
  );
}
