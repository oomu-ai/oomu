"use client";

import {
  useEffect,
  useId,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import type { RoutineTranslate } from "./routineLabels";

export function RoutineDeleteDialog({
  busy,
  error,
  label,
  onCancel,
  onConfirm,
  t,
}: {
  busy: boolean;
  error: string;
  label: string;
  onCancel: () => void;
  onConfirm: () => void;
  t: RoutineTranslate;
}) {
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (busy) {
      dialogRef.current?.focus();
    } else {
      cancelRef.current?.focus();
    }
  }, [busy]);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const buttons = Array.from(
      dialogRef.current?.querySelectorAll<HTMLButtonElement>(
        "button:not(:disabled)",
      ) ?? [],
    );
    if (buttons.length === 0) {
      event.preventDefault();
      dialogRef.current?.focus();
      return;
    }
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6">
      <div
        aria-busy={busy}
        aria-describedby={descriptionId}
        aria-labelledby={titleId}
        aria-modal="true"
        className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-6 shadow-[var(--shadow-raised)]"
        onKeyDown={handleKeyDown}
        ref={dialogRef}
        role="dialog"
        tabIndex={-1}
      >
        <h3 className="text-lg font-semibold" id={titleId}>
          {t("routines.delete_title", { name: label })}
        </h3>
        <div
          className="mt-3 grid gap-2 text-sm text-[var(--foreground-muted)]"
          id={descriptionId}
        >
          <p>{t("routines.delete_warning")}</p>
          <p>{t("routines.delete_preserved")}</p>
        </div>
        {error ? (
          <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">
            {error}
          </p>
        ) : null}
        <div className="mt-6 flex justify-end gap-2">
          <button
            className="rounded border px-3 py-2 text-sm"
            disabled={busy}
            onClick={onCancel}
            ref={cancelRef}
            type="button"
          >
            {t("common.cancel")}
          </button>
          <button
            className="rounded bg-[var(--destructive)] px-3 py-2 text-sm font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-50"
            disabled={busy}
            onClick={onConfirm}
            type="button"
          >
            {busy ? t("routines.deleting") : t("routines.delete_confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
