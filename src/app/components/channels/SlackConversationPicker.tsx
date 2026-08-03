"use client";

import { useI18n } from "@/context/I18nContext";
import type { SlackConversation } from "../integrations/integrationClient";

type Props = {
  conversations: SlackConversation[];
  onChange: (ids: string) => void;
  onRetry: () => void;
  selectedIds: string;
  state: "idle" | "loading" | "ready" | "error";
};

export function SlackConversationPicker({
  conversations,
  onChange,
  onRetry,
  selectedIds,
  state,
}: Props) {
  const { t } = useI18n();
  const selected = new Set(selectedIds.split("\n").map((value) => value.trim()).filter(Boolean));

  return <fieldset>
    <legend className="text-sm font-medium">{t("channels.slack_allowlist_label")}</legend>
    <p className="mt-1.5 text-xs leading-5 text-[var(--foreground-muted)]">{t("channels.slack_allowlist_help")}</p>
    {state === "loading" ? <p aria-live="polite" className="mt-3 text-sm text-[var(--foreground-muted)]">{t("channels.slack_conversations_loading")}</p> : null}
    {state === "error" ? <div className="mt-3 rounded-[var(--radius-sm)] border border-[var(--destructive)] bg-[var(--destructive-background)] p-3" role="alert">
      <p className="text-sm text-[var(--destructive)]">{t("channels.slack_conversations_failed")}</p>
      <button className="mt-2 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-semibold" onClick={onRetry} type="button">{t("common.try_again")}</button>
    </div> : null}
    {state === "ready" && conversations.length === 0 ? <p className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-3 text-sm text-[var(--foreground-muted)]">{t("channels.slack_conversations_empty")}</p> : null}
    {conversations.length ? <div className="mt-3 max-h-72 space-y-1 overflow-y-auto rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-2">
      {conversations.map((conversation) => {
        const checked = selected.has(conversation.id);
        const kind = t(`channels.slack_kind_${conversation.kind}`);
        const prefix = conversation.kind.includes("channel") ? "#" : "";
        return <label className="flex cursor-pointer items-center gap-3 rounded-[var(--radius-sm)] px-3 py-2.5 hover:bg-[var(--fill-hover)]" key={conversation.id}>
          <input
            checked={checked}
            className="h-4 w-4 accent-[var(--accent)]"
            onChange={() => {
              const next = new Set(selected);
              if (checked) next.delete(conversation.id); else next.add(conversation.id);
              onChange([...next].join("\n"));
            }}
            type="checkbox"
          />
          <span className="min-w-0">
            <span className="block truncate text-sm font-medium">{conversation.name ? `${prefix}${conversation.name}` : kind}</span>
            <span className="block text-xs text-[var(--foreground-subtle)]">{conversation.name ? kind : conversation.id}</span>
          </span>
        </label>;
      })}
    </div> : null}
  </fieldset>;
}
