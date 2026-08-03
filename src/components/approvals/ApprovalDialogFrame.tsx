"use client";

import {
  useEffect,
  useId,
  useRef,
  type KeyboardEvent,
  type ReactNode,
} from "react";

type ApprovalDialogFrameProps = {
  children: ReactNode;
  description: ReactNode;
  eyebrow: ReactNode;
  footer: ReactNode;
  id?: string;
  maxWidthClassName?: string;
  onDismiss: () => void;
  title: ReactNode;
};

const FOCUSABLE =
  'button:not([disabled]), summary, [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function ApprovalDialogFrame({
  children,
  description,
  eyebrow,
  footer,
  id,
  maxWidthClassName = "max-w-lg",
  onDismiss,
  title,
}: ApprovalDialogFrameProps) {
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const dialog = dialogRef.current;
    const firstControl =
      dialog?.querySelector<HTMLElement>(
        "[data-approval-initial-focus]:not([disabled]):not([aria-disabled=\"true\"])",
      ) ?? dialog?.querySelector<HTMLElement>(FOCUSABLE);
    (firstControl ?? dialog)?.focus();

    return () => {
      const previousFocus = previousFocusRef.current;
      queueMicrotask(() => {
        const activeElement = document.activeElement;
        if (
          previousFocus?.isConnected &&
          (!activeElement || activeElement === document.body)
        ) {
          previousFocus.focus();
        }
      });
    };
  }, []);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onDismiss();
      return;
    }
    if (event.key !== "Tab") {
      return;
    }

    const controls = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [],
    ).filter((control) => !control.hasAttribute("disabled"));
    if (controls.length === 0) {
      event.preventDefault();
      dialogRef.current?.focus();
      return;
    }
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/35 p-3 backdrop-blur-[2px] sm:items-center sm:p-4">
      <div
        aria-describedby={descriptionId}
        aria-labelledby={titleId}
        aria-modal="true"
        className={`flex max-h-[calc(100vh-1.5rem)] w-full flex-col overflow-hidden rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground)] shadow-2xl sm:max-h-[min(760px,calc(100vh-2rem))] ${maxWidthClassName}`}
        id={id}
        onKeyDown={handleKeyDown}
        ref={dialogRef}
        role="dialog"
        tabIndex={-1}
      >
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6">
          <p className="text-[11px] font-medium uppercase tracking-[0.08em] text-[var(--foreground-muted)]">
            {eyebrow}
          </p>
          <h2 className="mt-2 text-xl font-semibold tracking-tight" id={titleId}>
            {title}
          </h2>
          <div className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]" id={descriptionId}>
            {description}
          </div>
          {children}
        </div>
        <div className="flex flex-col gap-2 border-t border-[var(--border-soft)] bg-[var(--accent-background)] p-4 sm:flex-row sm:justify-end">
          {footer}
        </div>
      </div>
    </div>
  );
}
