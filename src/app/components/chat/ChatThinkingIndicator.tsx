"use client";

import { useI18n } from "@/context/I18nContext";

export function ChatThinkingIndicator({
  agentName,
  visible,
}: {
  agentName: string | null;
  visible: boolean;
}) {
  const { t } = useI18n();
  if (!visible) return null;
  return (
    <div
      aria-live="polite"
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--background)] px-4 py-3 text-sm font-medium text-[var(--foreground-muted)]"
      role="status"
    >
      <span className="inline-flex items-center gap-2">
        <span className="h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-[var(--accent)] border-t-transparent" />
        <span>{t("chat.thinking_named", { name: agentName ?? t("chat.an_agent") })}</span>
      </span>
    </div>
  );
}
