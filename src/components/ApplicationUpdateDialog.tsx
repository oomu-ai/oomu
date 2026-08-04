"use client";

import { useEffect, useRef, type ReactNode } from "react";
import { useI18n } from "@/context/I18nContext";
import { formatUpdateBytes, progressPercent, type ApplicationUpdateView } from "@/lib/applicationUpdates";
import { OomuRaven } from "./OomuRaven";

type ApplicationUpdateDialogProps = {
  view: ApplicationUpdateView | null;
  presentationBlocked?: boolean;
  onCheck: () => void;
  onDismiss: () => void;
  onInstall: () => void;
  onOpenFullNotes: () => void;
  onRemind: () => void;
  onRestart: () => void;
  onSkip: () => void;
};

type UpdateTranslator = ReturnType<typeof useI18n>["t"];

const focusableSelector = "button:not([disabled]), [href], [tabindex]:not([tabindex='-1'])";

export function ApplicationUpdateDialog({
  view,
  presentationBlocked = false,
  onCheck,
  onDismiss,
  onInstall,
  onOpenFullNotes,
  onRemind,
  onRestart,
  onSkip,
}: ApplicationUpdateDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!view || presentationBlocked) return;
    const activeView = view;
    const dialog = dialogRef.current;
    const focusable = dialog?.querySelectorAll<HTMLElement>(focusableSelector);
    focusable?.[0]?.focus();

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        if (activeView.status === "update_available") {
          event.preventDefault();
          onRemind();
        } else if (["up_to_date", "failed", "ready_to_restart"].includes(activeView.status)) {
          event.preventDefault();
          onDismiss();
        }
        return;
      }
      if (event.key !== "Tab" || !dialog) return;
      const items = [...dialog.querySelectorAll<HTMLElement>(focusableSelector)];
      if (items.length === 0) return;
      const first = items[0];
      const last = items.at(-1)!;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onDismiss, onRemind, presentationBlocked, view]);

  if (!view || presentationBlocked) return null;

  const percent = progressPercent(view.downloadedBytes, view.totalBytes);
  const title = titleForStatus(view, t);
  const description = descriptionForStatus(view, t);
  const working = ["checking", "downloading", "verifying"].includes(view.status);

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/25 p-6 backdrop-blur-[2px]">
      <div
        aria-describedby="application-update-description"
        aria-labelledby="application-update-title"
        aria-modal="true"
        className="w-full max-w-[520px] rounded-[22px] border border-[var(--border)] bg-[var(--background)] p-7 shadow-2xl"
        data-oomu-application-update-dialog
        ref={dialogRef}
        role="dialog"
      >
        <div className="flex items-start gap-4">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-[var(--accent-background)] text-[var(--foreground)]">
            <OomuRaven className="h-7 w-7" />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-xl font-semibold tracking-[-0.02em]" id="application-update-title">
              {title}
            </h2>
            <p className="mt-1.5 text-sm leading-6 text-[var(--foreground-muted)]" id="application-update-description">
              {description}
            </p>
          </div>
        </div>

        <ReleaseNotes view={view} onOpen={onOpenFullNotes} t={t} />
        <DownloadProgress percent={percent} t={t} view={view} />
        <WorkingState t={t} view={view} working={working} />
        <DialogActions
          {...{ onCheck, onDismiss, onInstall, onRemind, onRestart, onSkip, t, view }}
        />
      </div>
    </div>
  );
}

function ReleaseNotes({ view, onOpen, t }: { view: ApplicationUpdateView; onOpen: () => void; t: UpdateTranslator }) {
  if (view.status !== "update_available" || !view.notes) return null;
  return (
    <section className="mt-6" aria-labelledby="application-update-whats-new">
      <h3 className="text-sm font-semibold" id="application-update-whats-new">{t("application_updates.whats_new")}</h3>
      <div className="mt-2 max-h-48 overflow-y-auto whitespace-pre-line rounded-[var(--radius-md)] bg-[var(--background-muted)] px-4 py-3 text-sm leading-6 text-[var(--foreground-muted)]">{view.notes}</div>
      {view.fullNotesAvailable ? (
        <button className="mt-3 text-sm font-medium text-[var(--accent)] hover:underline" onClick={onOpen} type="button">{t("application_updates.actions.full_notes")}</button>
      ) : null}
    </section>
  );
}

function DownloadProgress({ percent, t, view }: { percent: number | null; t: UpdateTranslator; view: ApplicationUpdateView }) {
  if (view.status !== "downloading") return null;
  const amount = view.totalBytes
    ? t("application_updates.progress", {
      downloaded: formatUpdateBytes(view.downloadedBytes),
      total: formatUpdateBytes(view.totalBytes),
    })
    : formatUpdateBytes(view.downloadedBytes);
  return (
    <div className="mt-6">
      <div className="mb-2 flex items-center justify-between text-xs font-medium text-[var(--foreground-muted)]">
        <span>{t("application_updates.downloading")}</span><span>{amount}</span>
      </div>
      <div aria-label={t("application_updates.progress_label")} aria-valuemax={100} aria-valuemin={0} aria-valuenow={percent ?? undefined} className="h-2 overflow-hidden rounded-full bg-[var(--fill-hover)]" role="progressbar">
        <div className={`h-full rounded-full bg-[var(--accent)] transition-[width] duration-300 motion-reduce:transition-none ${percent === null ? "w-1/3 animate-pulse motion-reduce:animate-none" : ""}`} style={percent === null ? undefined : { width: `${percent}%` }} />
      </div>
    </div>
  );
}

function WorkingState({ t, view, working }: { t: UpdateTranslator; view: ApplicationUpdateView; working: boolean }) {
  if (!working || view.status === "downloading") return null;
  return (
    <div className="mt-6 flex items-center gap-3 text-sm text-[var(--foreground-muted)]" role="status">
      <span aria-hidden="true" className="h-4 w-4 animate-spin rounded-full border-2 border-[var(--border-strong)] border-t-[var(--accent)] motion-reduce:animate-none" />
      {view.status === "checking" ? t("application_updates.checking") : t("application_updates.verifying")}
    </div>
  );
}

type DialogActionsProps = Omit<ApplicationUpdateDialogProps, "presentationBlocked" | "onOpenFullNotes"> & { t: UpdateTranslator };

function DialogActions({ view, onCheck, onDismiss, onInstall, onRemind, onRestart, onSkip, t }: DialogActionsProps) {
  if (!view) return null;
  return (
    <div className="mt-7 flex flex-wrap items-center justify-end gap-2">
      {view.status === "up_to_date" ? <SecondaryButton onClick={onDismiss}>{t("application_updates.actions.done")}</SecondaryButton> : null}
      {view.status === "failed" ? <>{view.retryable ? <PrimaryButton onClick={onCheck}>{t("application_updates.actions.try_again")}</PrimaryButton> : null}<SecondaryButton onClick={onDismiss}>{t("application_updates.actions.done")}</SecondaryButton></> : null}
      {view.status === "update_available" ? <><PrimaryButton onClick={onInstall}>{t("application_updates.actions.install")}</PrimaryButton><SecondaryButton onClick={onRemind}>{t("application_updates.actions.remind")}</SecondaryButton><button className="basis-full pt-2 text-center text-xs font-medium text-[var(--foreground-muted)] hover:text-[var(--foreground)]" onClick={onSkip} type="button">{t("application_updates.actions.skip")}</button></> : null}
      {view.status === "ready_to_restart" ? <><PrimaryButton onClick={onRestart}>{t("application_updates.actions.restart")}</PrimaryButton><SecondaryButton onClick={onDismiss}>{t("application_updates.actions.later")}</SecondaryButton></> : null}
    </div>
  );
}

function titleForStatus(view: ApplicationUpdateView, t: UpdateTranslator) {
  if (view.status === "up_to_date") return t("application_updates.up_to_date_title");
  if (view.status === "update_available") return t("application_updates.available_title");
  if (view.status === "ready_to_restart") return t("application_updates.restart_title");
  if (view.status === "failed") {
    return ["download_failed", "signature_invalid", "install_failed"].includes(view.publicCode ?? "")
      ? t("application_updates.failed_title")
      : t("application_updates.check_failed_title");
  }
  return t("application_updates.checking_title");
}

function descriptionForStatus(view: ApplicationUpdateView, t: UpdateTranslator) {
  if (view.status === "up_to_date") return t("application_updates.up_to_date_body", { version: view.currentVersion });
  if (view.status === "update_available") {
    return t("application_updates.available_body", {
      availableVersion: view.availableVersion ?? "",
      currentVersion: view.currentVersion,
    });
  }
  if (view.status === "downloading") return t("application_updates.downloading_body");
  if (view.status === "verifying") return t("application_updates.verifying_body");
  if (view.status === "ready_to_restart") return t("application_updates.restart_body");
  if (view.status === "failed") {
    return view.publicCode === "signature_invalid"
      ? t("application_updates.signature_failed_body")
      : view.publicCode === "install_failed"
        ? t("application_updates.install_failed_body")
        : view.publicCode === "download_failed"
          ? t("application_updates.failed_body")
          : t("application_updates.check_failed_body");
  }
  return t("application_updates.checking_body");
}

function PrimaryButton({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return <button className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-white hover:brightness-95" onClick={onClick} type="button">{children}</button>;
}

function SecondaryButton({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-4 py-2 text-sm font-medium hover:bg-[var(--fill-hover)]" onClick={onClick} type="button">{children}</button>;
}
