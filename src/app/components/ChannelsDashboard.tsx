"use client";

import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import {
  ChannelSetupForm,
  EMPTY_CHANNEL_DRAFT,
  type ChannelDraft,
} from "./channels/ChannelSetupForm";
import {
  useSlackAuthorization,
  type SlackAuthorizationAttempt,
} from "./channels/useSlackAuthorization";
import { useChannelDirectory } from "./channels/useChannelDirectory";
import { ConnectorAvailabilityNotice } from "./integrations/ConnectorAvailabilityNotice";
import {
  integrationApi,
  type ConnectorAccount,
  type SlackConversation,
} from "./integrations/integrationClient";
import {
  hasChannelError,
  isChannelReady,
  type ChannelPlatform,
  type ChannelStatus,
} from "./routines/channelReadiness";

const PLATFORM_META: ReadonlyArray<{ id: ChannelPlatform }> = [
  { id: "telegram" },
  { id: "discord" },
  { id: "slack" },
];

const READY_ACCOUNT_STATES = new Set(["authorized", "reachable", "configured"]);

function statusTone(status?: ChannelStatus) {
  if (status && hasChannelError(status)) {
    return "border-[var(--destructive)] bg-[var(--destructive-background)] text-[var(--destructive)]";
  }
  if (status && isChannelReady(status)) {
    return "border-[var(--success)] bg-[var(--success-background)] text-[var(--success)]";
  }
  return "border-[var(--border-strong)] bg-[var(--accent-background)] text-[var(--foreground-muted)]";
}

function splitAllowlist(value: string) {
  return value.split(/[\n,]/).map((item) => item.trim()).filter(Boolean);
}

function accountHasMessaging(account?: ConnectorAccount) {
  return Boolean(account?.grantedScopes.includes("chat:write"));
}

function accountRank(account: ConnectorAccount) {
  return READY_ACCOUNT_STATES.has(account.connectionState) ? 1 : 0;
}

function preferredSlackAccount(accounts: ConnectorAccount[]) {
  return accounts
    .filter((account) => account.manifestId === "slack")
    .sort((left, right) => accountRank(right) - accountRank(left))[0];
}

export function slackMessagingUnavailableReason(
  manifest: Awaited<ReturnType<typeof integrationApi.manifests>>[number] | undefined,
) {
  if (!manifest) return undefined;
  if (!manifest.supported) return manifest.availabilityReasonCode;
  const messaging = manifest.operationGrants?.find((grant) => grant.operation === "slack.messaging");
  return messaging?.available === false ? messaging.unavailableReasonCode ?? undefined : undefined;
}

export function ChannelsDashboard() {
  const { t } = useI18n();
  const { accounts, load, loadFailed, manifests, statuses } = useChannelDirectory();
  const [selectedPlatform, setSelectedPlatform] = useState<ChannelPlatform | null>(null);
  const [draft, setDraft] = useState<ChannelDraft>({ ...EMPTY_CHANNEL_DRAFT });
  const [busyPlatform, setBusyPlatform] = useState<ChannelPlatform | null>(null);
  const [messageKey, setMessageKey] = useState<string | null>(null);
  const [slackConversations, setSlackConversations] = useState<SlackConversation[]>([]);
  const [slackConversationsState, setSlackConversationsState] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [slackAuthorization, setSlackAuthorization] = useState<SlackAuthorizationAttempt | null>(null);

  const finishSlackAuthorization = useCallback(async (
    outcome: "complete" | "denied" | "expired" | "workspace_restricted" | "failed",
  ) => {
    setSlackAuthorization(null);
    if (outcome !== "complete") {
      const key = outcome === "denied"
        ? "channels.slack_authorization_denied"
        : outcome === "expired"
          ? "channels.slack_authorization_timed_out"
          : outcome === "workspace_restricted"
            ? "channels.slack_authorization_workspace_restricted"
            : "channels.slack_authorization_failed";
      setMessageKey(key);
      return;
    }
    const refreshed = await load();
    const account = refreshed ? preferredSlackAccount(refreshed.accounts) : undefined;
    if (!refreshed || !accountHasMessaging(account)) {
      setMessageKey("channels.slack_authorization_failed");
      return;
    }
    const status = refreshed.statuses.find((item) => item.platform === "slack");
    setDraft({
      ...EMPTY_CHANNEL_DRAFT,
      ownerId: status?.ownerId?.trim() || account?.accountId || "",
      allowlistChannels: status?.allowedChannelIds?.join("\n") || "",
    });
    setMessageKey(null);
    setSelectedPlatform("slack");
  }, [load]);
  useSlackAuthorization(slackAuthorization, finishSlackAuthorization);

  const statusByPlatform = useMemo(() => Object.fromEntries(
    statuses.map((status) => [status.platform, status]),
  ) as Partial<Record<ChannelPlatform, ChannelStatus>>, [statuses]);
  const slackAccount = useMemo(() => preferredSlackAccount(accounts), [accounts]);
  const slackManifest = useMemo(
    () => manifests.find((manifest) => manifest.manifestId === "slack"),
    [manifests],
  );
  const slackUnavailableReason = slackMessagingUnavailableReason(slackManifest);
  const slackMessagingEnabled = accountHasMessaging(slackAccount);
  const slackConnectorId = slackAccount?.connectorId;
  const loadSlackConversations = useCallback(async () => {
    if (!slackConnectorId || !slackMessagingEnabled) {
      setSlackConversations([]);
      setSlackConversationsState("idle");
      return;
    }
    setSlackConversationsState("loading");
    try {
      setSlackConversations(await integrationApi.slackConversations(slackConnectorId));
      setSlackConversationsState("ready");
    } catch {
      setSlackConversationsState("error");
    }
  }, [slackConnectorId, slackMessagingEnabled]);

  useEffect(() => {
    if (selectedPlatform !== "slack") return;
    const task = window.setTimeout(() => void loadSlackConversations(), 0);
    return () => window.clearTimeout(task);
  }, [loadSlackConversations, selectedPlatform]);

  function openConfiguration(platform: ChannelPlatform) {
    setDraft({
      ...EMPTY_CHANNEL_DRAFT,
      ownerId: statusByPlatform[platform]?.ownerId?.trim()
        || (platform === "slack" ? slackAccount?.accountId : undefined)
        || "",
      allowlistChannels: statusByPlatform[platform]?.allowedChannelIds?.join("\n") || "",
    });
    setMessageKey(null);
    setSelectedPlatform(platform);
  }

  function closeConfiguration() {
    setDraft({ ...EMPTY_CHANNEL_DRAFT });
    setMessageKey(null);
    setSelectedPlatform(null);
    setSlackConversations([]);
    setSlackConversationsState("idle");
  }

  async function startSlackMessaging() {
    setBusyPlatform("slack");
    setMessageKey(null);
    try {
      const startedAtMs = Date.now();
      const response = await integrationApi.connect("slack", slackAccount?.connectorId, ["slack.messaging"]);
      setSlackAuthorization({
        connectorId: response.connectorId,
        expiresAtMs: response.expiresAtMs,
        startedAtMs,
      });
      setMessageKey("channels.slack_authorization_opened");
      await load();
    } catch {
      setMessageKey("channels.slack_authorization_failed");
    } finally {
      setBusyPlatform(null);
    }
  }

  async function saveConfiguration(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedPlatform) return;
    const status = statusByPlatform[selectedPlatform];
    const ownerId = draft.ownerId.trim() || status?.ownerId?.trim() || "";
    if (!ownerId) {
      setMessageKey("channels.owner_required");
      return;
    }
    if (selectedPlatform === "slack" && !slackAccount) {
      setMessageKey("channels.slack_connection_required");
      return;
    }
    const allowlistChannels = splitAllowlist(draft.allowlistChannels);
    if (selectedPlatform === "slack" && allowlistChannels.length === 0) {
      setMessageKey("channels.slack_allowlist_required");
      return;
    }
    if (selectedPlatform !== "slack" && !status?.isActive && !draft.secret.trim()) {
      setMessageKey("channels.secret_required");
      return;
    }

    const credentials = selectedPlatform === "telegram"
      ? { botToken: draft.secret.trim(), ownerChatId: ownerId }
      : selectedPlatform === "discord"
        ? { apiKey: draft.secret.trim(), allowlistChannels }
        : { connectorId: slackAccount?.connectorId, allowlistChannels };
    setBusyPlatform(selectedPlatform);
    setMessageKey("channels.status_saving");
    try {
      await invoke("save_channel_config", {
        request: {
          platform: selectedPlatform,
          isActive: true,
          ...(draft.secret.trim() || selectedPlatform === "slack"
            ? { credentialsJson: JSON.stringify(credentials) }
            : {}),
          ownerId,
        },
      });
      const next = await load();
      const saved = next?.statuses.find((item) => item.platform === selectedPlatform);
      setMessageKey(saved && hasChannelError(saved)
        ? "channels.connection_failed"
        : "channels.connection_saved");
      if (saved && !hasChannelError(saved)) closeConfiguration();
    } catch {
      setMessageKey("channels.connection_failed");
    } finally {
      setBusyPlatform(null);
    }
  }

  async function disable(platform: ChannelPlatform) {
    setBusyPlatform(platform);
    setMessageKey(null);
    try {
      await invoke("save_channel_config", { request: { platform, isActive: false } });
      await load();
    } catch {
      setMessageKey("channels.disable_failed");
    } finally {
      setBusyPlatform(null);
    }
  }

  return <section className="h-full overflow-y-auto bg-[var(--background)] text-[var(--foreground)]">
    <div className="mx-auto w-full max-w-[70rem] px-6 py-8">
      <header className="max-w-3xl">
        <h1 className="text-2xl font-semibold">{t("channels.title")}</h1>
        <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">{t("channels.subtitle")}</p>
      </header>

      {loadFailed ? <p className="mt-5 rounded-[var(--radius-md)] border border-[var(--destructive)] bg-[var(--destructive-background)] p-4 text-sm text-[var(--destructive)]" role="alert">{t("channels.load_failed")}</p> : null}
      {messageKey && !selectedPlatform ? <p className="mt-5 text-sm text-[var(--foreground-muted)]" role="status">{t(messageKey)}</p> : null}

      <div className="mt-7 grid gap-4 lg:grid-cols-3">
        {PLATFORM_META.map(({ id }) => {
          const status = statusByPlatform[id];
          const ready = Boolean(status && isChannelReady(status));
          const active = Boolean(status?.isActive);
          const busy = busyPlatform === id;
          const slackNeedsInstall = id === "slack" && !slackAccount;
          const slackNeedsUpgrade = id === "slack" && Boolean(slackAccount) && !slackMessagingEnabled;
          const stateKey = slackNeedsInstall
            ? "channels.state_not_connected"
            : slackNeedsUpgrade
              ? "channels.slack_read_only"
              : ready
                ? "channels.state_configured"
                : active
                  ? "channels.state_connecting"
                  : "channels.state_unconfigured";

          return <article className="flex min-h-72 flex-col rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--background)] p-5 shadow-[var(--shadow-card)]" key={id}>
            <p className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">{t(`channels.eyebrow_${id}`)}</p>
            <h2 className="mt-2 text-xl font-semibold">{t(`channels.platform_${id}`)}</h2>
            <p className="mt-3 flex-1 text-sm leading-6 text-[var(--foreground-muted)]">{t(`channels.description_${id}`)}</p>
            <div className={`mt-5 inline-flex w-fit items-center rounded-full border px-2.5 py-1 text-xs font-semibold ${statusTone(status)}`}>{t(stateKey)}</div>
            {id === "slack" && slackUnavailableReason ? <ConnectorAvailabilityNotice
              reasonCode={slackUnavailableReason}
              service={t("channels.platform_slack")}
            /> : <div className="mt-5 flex flex-wrap gap-2">
              {slackNeedsInstall || slackNeedsUpgrade ? <button
                aria-busy={busy}
                className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-50"
                data-action-state={busy ? "working" : "idle"}
                disabled={busy}
                onClick={() => void startSlackMessaging()}
                type="button"
              >{busy ? t("channels.opening_slack") : t(slackNeedsInstall ? "channels.install_slack" : "channels.turn_on_messaging")}</button> : <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)]"
                onClick={() => openConfiguration(id)}
                type="button"
              >{t("channels.configure")}</button>}
              {active ? <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-4 py-2 text-sm font-semibold disabled:opacity-50" disabled={busy} onClick={() => void disable(id)} type="button">{t("channels.disable")}</button> : null}
            </div>}
          </article>;
        })}
      </div>
    </div>

    {selectedPlatform ? <ChannelSetupForm
      account={selectedPlatform === "slack" ? slackAccount : undefined}
      busy={busyPlatform === selectedPlatform}
      draft={draft}
      messageKey={messageKey}
      onCancel={closeConfiguration}
      onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))}
      onSubmit={(event) => void saveConfiguration(event)}
      platform={selectedPlatform}
      slackConversations={slackConversations}
      slackConversationsState={slackConversationsState}
      onRetrySlackConversations={() => void loadSlackConversations()}
      status={statusByPlatform[selectedPlatform]}
    /> : null}
  </section>;
}
