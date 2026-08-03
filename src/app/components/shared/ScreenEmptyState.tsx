"use client";

import type { ReactNode } from "react";

export function ScreenEmptyState({
  actionLabel,
  body,
  className = "",
  icon,
  onAction,
  title,
}: {
  actionLabel?: string;
  body: string;
  className?: string;
  icon: ReactNode;
  onAction?: () => void;
  title: string;
}) {
  return (
    <div className={`rounded-[var(--radius-sm)] border border-dashed border-[var(--border-soft)] p-6 text-center ${className}`.trim()}>
      <div className="mx-auto flex w-fit text-[var(--foreground-muted)]">{icon}</div>
      <h2 className="mt-4 text-sm font-semibold text-[var(--foreground)]">{title}</h2>
      <p className="mx-auto mt-2 max-w-xl text-sm leading-6 text-[var(--foreground-muted)]">{body}</p>
      {actionLabel && onAction ? (
        <button className="mt-4 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-opacity hover:opacity-90" onClick={onAction} type="button">
          {actionLabel}
        </button>
      ) : null}
    </div>
  );
}
