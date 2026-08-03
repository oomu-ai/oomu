"use client";

import { useEffect, useRef, type FormEvent } from "react";
import { useI18n } from "@/context/I18nContext";
import type { ConnectorAccount, SlackConversation } from "../integrations/integrationClient";
import type { ChannelPlatform, ChannelStatus } from "../routines/channelReadiness";
import { SlackConversationPicker } from "./SlackConversationPicker";

export type ChannelDraft = {
  ownerId: string;
  secret: string;
  allowlistChannels: string;
};

export const EMPTY_CHANNEL_DRAFT: ChannelDraft = {
  ownerId: "",
  secret: "",
  allowlistChannels: "",
};

type Props = {
  account?: ConnectorAccount;
  busy: boolean;
  draft: ChannelDraft;
  messageKey: string | null;
  onCancel: () => void;
  onChange: (patch: Partial<ChannelDraft>) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  platform: ChannelPlatform;
  slackConversations: SlackConversation[];
  slackConversationsState: "idle" | "loading" | "ready" | "error";
  onRetrySlackConversations: () => void;
  status?: ChannelStatus;
};

function TextField({
  help,
  id,
  label,
  onChange,
  placeholder,
  secret = false,
  value,
}: {
  help?: string;
  id: string;
  label: string;
  onChange: (value: string) => void;
  placeholder: string;
  secret?: boolean;
  value: string;
}) {
  return <div>
    <label className="text-sm font-medium" htmlFor={id}>{label}</label>
    <input
      aria-describedby={help ? `${id}-help` : undefined}
      autoComplete="off"
      className="mt-2 w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm outline-none focus:border-[var(--accent)]"
      id={id}
      onChange={(event) => onChange(event.currentTarget.value)}
      placeholder={placeholder}
      type={secret ? "password" : "text"}
      value={value}
    />
    {help ? <p className="mt-1.5 text-xs leading-5 text-[var(--foreground-muted)]" id={`${id}-help`}>{help}</p> : null}
  </div>;
}

export function ChannelSetupForm({
  account,
  busy,
  draft,
  messageKey,
  onCancel,
  onChange,
  onSubmit,
  platform,
  slackConversations,
  slackConversationsState,
  onRetrySlackConversations,
  status,
}: Props) {
  const { t } = useI18n();
  const title = t(`channels.platform_${platform}`);
  const savedOwner = status?.ownerId?.trim() ?? "";
  const cancelRef = useRef<HTMLButtonElement>(null);
  const onCancelRef = useRef(onCancel);
  useEffect(() => {
    onCancelRef.current = onCancel;
  }, [onCancel]);

  useEffect(() => {
    cancelRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onCancelRef.current();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy]);

  return <div className="fixed inset-0 z-50 flex justify-end bg-black/30" role="presentation">
    <form
      aria-label={t("channels.drawer_title", { platform: title })}
      aria-modal="true"
      className="flex h-full w-full max-w-[34rem] flex-col bg-[var(--background)] shadow-2xl"
      onSubmit={onSubmit}
      role="dialog"
    >
      <header className="border-b border-[var(--border-soft)] px-7 py-6">
        <p className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">{t(`channels.eyebrow_${platform}`)}</p>
        <h2 className="mt-2 text-2xl font-semibold">{t("channels.drawer_title", { platform: title })}</h2>
        <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">{t(`channels.${platform}_setup_help`)}</p>
      </header>

      <div className="flex-1 space-y-5 overflow-y-auto px-7 py-6">
        {platform === "slack" && account ? <div className="rounded-[var(--radius-md)] border border-[var(--success)] bg-[var(--success-background)] p-4">
          <p className="text-sm font-semibold">{t("channels.slack_workspace_connected")}</p>
          <p className="mt-1 text-sm text-[var(--foreground-muted)]">{account.accountLabel}</p>
        </div> : null}

        {platform === "telegram" ? <TextField
          id="telegram-token"
          label={t("channels.telegram_token_label")}
          onChange={(secret) => onChange({ secret })}
          placeholder={status?.isActive ? t("channels.saved_secret_placeholder") : t("channels.telegram_token_placeholder")}
          secret
          value={draft.secret}
        /> : null}

        {platform === "discord" ? <TextField
          id="discord-token"
          label={t("channels.discord_apikey_label")}
          onChange={(secret) => onChange({ secret })}
          placeholder={status?.isActive ? t("channels.saved_secret_placeholder") : t("channels.discord_apikey_placeholder")}
          secret
          value={draft.secret}
        /> : null}

        <TextField
          help={platform === "slack" ? t("channels.slack_owner_help") : t("channels.owner_help")}
          id={`${platform}-owner`}
          label={t(`channels.${platform}_owner_label`)}
          onChange={(ownerId) => onChange({ ownerId })}
          placeholder={savedOwner || t(`channels.${platform}_owner_placeholder`)}
          value={draft.ownerId}
        />

        {platform === "discord" ? <div>
          <label className="text-sm font-medium" htmlFor={`${platform}-channels`}>{t(`channels.${platform}_allowlist_label`)}</label>
          <textarea
            aria-describedby={`${platform}-channels-help`}
            className="mt-2 min-h-28 w-full resize-y rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm outline-none focus:border-[var(--accent)]"
            id={`${platform}-channels`}
            onChange={(event) => onChange({ allowlistChannels: event.currentTarget.value })}
            placeholder={t(`channels.${platform}_channel_placeholder`)}
            value={draft.allowlistChannels}
          />
          <p className="mt-1.5 text-xs leading-5 text-[var(--foreground-muted)]" id={`${platform}-channels-help`}>{t(`channels.${platform}_allowlist_help`)}</p>
        </div> : null}

        {platform === "slack" ? <SlackConversationPicker
          conversations={slackConversations}
          onChange={(allowlistChannels) => onChange({ allowlistChannels })}
          onRetry={onRetrySlackConversations}
          selectedIds={draft.allowlistChannels}
          state={slackConversationsState}
        /> : null}

        {messageKey ? <p aria-live="polite" className={messageKey.includes("failed") || messageKey.includes("required") ? "text-sm text-[var(--destructive)]" : "text-sm text-[var(--foreground-muted)]"} role={messageKey.includes("failed") || messageKey.includes("required") ? "alert" : "status"}>{t(messageKey)}</p> : null}
      </div>

      <footer className="flex justify-end gap-3 border-t border-[var(--border-soft)] px-7 py-5">
        <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-4 py-2 text-sm font-semibold hover:bg-[var(--fill-hover)]" disabled={busy} onClick={onCancel} ref={cancelRef} type="button">{t("common.cancel")}</button>
        <button aria-busy={busy} className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-50" data-action-state={busy ? "working" : "idle"} disabled={busy} type="submit">{busy ? t("channels.saving") : t("channels.save")}</button>
      </footer>
    </form>
  </div>;
}
