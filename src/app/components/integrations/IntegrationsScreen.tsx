"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useApprovalDialogTurn } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { projectApi, type ProjectRecord } from "../projects/projectClient";
import { ConnectorAvailabilityNotice } from "./ConnectorAvailabilityNotice";
import { ConnectorProjectScopeControls } from "./ConnectorProjectScopeControls";
import {
  integrationApi,
  type CapabilityHealth,
  type ConnectorAccount,
  type ConnectorCapabilityGrant,
  type ConnectorManifest,
} from "./integrationClient";
import { Microsoft365ControlPanel } from "./microsoft365/Microsoft365ControlPanel";
import { isMicrosoft365Manifest } from "./microsoft365/microsoft365Client";
import {
  useConnectorOAuthFlow,
  type ConnectorOAuthTarget,
} from "./useConnectorOAuthFlow";

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

type IntegrationActionKind = "check" | "remove" | "reconnect";
type IntegrationActionState = {
  connectorId: string;
  kind: IntegrationActionKind;
  state: "working" | "success" | "error";
  messageKey: string;
};

const READY_CONNECTION_STATES = new Set(["authorized", "reachable"]);

const GOOGLE_CAPABILITY_GROUPS = [
  { operation: "gmail.search", labelKey: "gmail_read" },
  { operation: "gmail.draft", labelKey: "gmail_drafts" },
  { operation: "calendar.read", labelKey: "calendar_read" },
  { operation: "calendar.create", labelKey: "calendar_change" },
  { operation: "drive.search", labelKey: "drive_read" },
] as const;

export function visibleConnectorManifests(
  manifests: ConnectorManifest[],
  developerBuild = isDeveloperBuild,
) {
  return developerBuild
    ? manifests
    : manifests.filter((item) => item.manifestId !== "mcp_runtime");
}

export function genericConnectionSummaryKey(accounts: ConnectorAccount[]) {
  if (accounts.length === 0) return "integrations.ready_to_connect";
  if (accounts.every((account) => READY_CONNECTION_STATES.has(account.connectionState))) {
    return "integrations.connection_ready";
  }
  if (accounts.some((account) => account.connectionState === "configured")) {
    return "integrations.connection_finishing";
  }
  return "integrations.connection_attention";
}

export const GENERIC_SCOPE_LABEL_KEYS = {
  openid: "integrations.scopes.identify_account",
  email: "integrations.scopes.view_email_address",
  profile: "integrations.scopes.identify_account",
  "https://www.googleapis.com/auth/userinfo.email": "integrations.scopes.view_email_address",
  "https://www.googleapis.com/auth/userinfo.profile": "integrations.scopes.identify_account",
  "https://www.googleapis.com/auth/gmail.readonly": "integrations.scopes.read_gmail",
  "https://www.googleapis.com/auth/gmail.compose": "integrations.scopes.create_gmail_drafts",
  "https://www.googleapis.com/auth/calendar.readonly": "sprint_299.google.calendar_read",
  "https://www.googleapis.com/auth/calendar.events": "sprint_299.google.calendar_change",
  "https://www.googleapis.com/auth/drive.readonly": "integrations.scopes.read_google_drive",
  "channels:history": "integrations.scopes.read_public_channel_messages",
  "channels:read": "integrations.scopes.view_public_channels",
  "groups:history": "integrations.scopes.read_private_channel_messages",
  "groups:read": "integrations.scopes.view_private_channels",
  "im:history": "integrations.scopes.read_direct_messages",
  "im:read": "integrations.scopes.view_direct_messages",
  "mpim:history": "integrations.scopes.read_group_messages",
  "mpim:read": "integrations.scopes.view_group_messages",
  "app_mentions:read": "integrations.scopes.read_app_mentions",
  "search:read": "integrations.scopes.search_slack",
  "chat:write": "integrations.scopes.send_slack_messages",
} as const satisfies Record<string, string>;

function genericScopeLabel(t: TranslateFn, scope: string) {
  const key = GENERIC_SCOPE_LABEL_KEYS[scope as keyof typeof GENERIC_SCOPE_LABEL_KEYS];
  return key
    ? t(key)
    : t("integrations.scopes.other", { scope: t("common.unknown") });
}

function genericDestinationLabels(t: TranslateFn, destinations: string[]) {
  return [...new Set(destinations.map((destination) => {
    let hostname = "";
    try {
      hostname = new URL(destination).hostname.replace(/^www\./i, "");
    } catch {
      hostname = /^(?:[a-z0-9](?:[a-z0-9-]{0,62})\.)+[a-z]{2,63}$/i.test(destination)
        ? destination
        : "";
    }
    if (/google(?:apis)?\.com$/i.test(hostname)) {
      return t("integrations.service_names.google_workspace");
    }
    if (/(?:^|\.)slack\.com$/i.test(hostname)) {
      return t("integrations.service_names.slack");
    }
    return hostname || t("common.unknown");
  }))];
}

export function IntegrationsScreen({
  onTurnOnMessaging,
  showIntroduction = true,
}: {
  onTurnOnMessaging?: () => void;
  showIntroduction?: boolean;
}) {
  const { t } = useI18n();
  const [manifests, setManifests] = useState<ConnectorManifest[]>([]);
  const [accounts, setAccounts] = useState<ConnectorAccount[]>([]);
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [selectedId, setSelectedId] = useState("google_workspace");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const actionInFlightRef = useRef(false);
  const selected = manifests.find((item) => item.manifestId === selectedId) ?? manifests[0];
  const selectedAccounts = useMemo(
    () => accounts.filter((item) => item.manifestId === selected?.manifestId),
    [accounts, selected],
  );
  const microsoftSelected = Boolean(selected && isMicrosoft365Manifest(selected.manifestId));

  const load = useCallback(async () => {
    try {
      const [nextManifests, nextAccounts, nextProjects] = await Promise.all([
        integrationApi.manifests(),
        integrationApi.accounts(),
        projectApi.list(),
      ]);
      const visibleManifests = visibleConnectorManifests(nextManifests);
      setManifests(visibleManifests);
      setAccounts(nextAccounts);
      setProjects(nextProjects);
      setSelectedId((current) => visibleManifests.some((item) => item.manifestId === current)
        ? current
        : (visibleManifests[0]?.manifestId ?? ""));
      setError("");
    } catch {
      setError("load_failed");
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void load(), 0);
    return () => window.clearTimeout(timer);
  }, [load]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      void integrationApi.accounts().then(setAccounts).catch(() => undefined);
    }, 4_000);
    return () => window.clearInterval(timer);
  }, []);

  async function act(action: () => Promise<unknown>, errorCode = "action_failed") {
    if (actionInFlightRef.current) return false;
    actionInFlightRef.current = true;
    setBusy(true);
    setError("");
    try {
      await action();
      await load();
      return true;
    } catch (cause) {
      const code = cause && typeof cause === "object" && "code" in cause
        ? String((cause as { code?: unknown }).code ?? "")
        : "";
      setError(code === "oauth_broker_unreachable" ? code : errorCode);
      return false;
    } finally {
      actionInFlightRef.current = false;
      setBusy(false);
    }
  }

  return <section className="grid h-full min-h-0 grid-cols-[19rem_minmax(0,1fr)] overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)]">
    <aside className="overflow-y-auto border-r border-[var(--border-soft)] p-5">
      {showIntroduction ? <><h1 className="text-lg font-semibold">{t("integrations.title")}</h1>
      <p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("integrations.subtitle")}</p></> : null}
      <div className={`${showIntroduction ? "mt-5 " : ""}grid gap-2`}>
        {manifests.map((item) => <button
          className={`rounded-[var(--radius-sm)] p-3 text-left ${selected?.manifestId === item.manifestId ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"}`}
          key={item.manifestId}
          onClick={() => setSelectedId(item.manifestId)}
          type="button"
        >
          <span className="block text-sm font-semibold">{connectorName(t, item.manifestId)}</span>
          <span className="mt-1.5 block text-sm leading-5 text-[var(--foreground-muted)]">
            {capabilitySummary(t, item.manifestId)}
          </span>
          <span className="mt-2 block text-xs font-medium text-[var(--foreground-subtle)]">
            {item.supported ? t("integrations.available") : t("integrations.unavailable")}
          </span>
        </button>)}
      </div>
    </aside>
    <div className="min-h-0 overflow-y-auto p-7">
      {selected ? microsoftSelected
        ? <div className="mx-auto max-w-3xl"><Microsoft365ControlPanel manifest={selected} projects={projects} /></div>
        : <GenericConnector
          accounts={selectedAccounts}
          busy={busy}
          error={error}
          manifest={selected}
          onAct={act}
          onRefresh={load}
          onTurnOnMessaging={onTurnOnMessaging}
          projects={projects}
        />
        : null}
    </div>
  </section>;
}

function GenericConnector({
  accounts,
  busy,
  error,
  manifest,
  onAct,
  onRefresh,
  onTurnOnMessaging,
  projects,
}: {
  accounts: ConnectorAccount[];
  busy: boolean;
  error: string;
  manifest: ConnectorManifest;
  onAct: (action: () => Promise<unknown>, errorCode?: string) => Promise<boolean>;
  onRefresh: () => Promise<void>;
  onTurnOnMessaging?: () => void;
  projects: ProjectRecord[];
}) {
  const { t } = useI18n();
  const [consentTarget, setConsentTarget] = useState<ConnectorOAuthTarget | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [health, setHealth] = useState<Record<string, CapabilityHealth>>({});
  const [actionState, setActionState] = useState<IntegrationActionState | null>(null);
  const [removeAccount, setRemoveAccount] = useState<ConnectorAccount | null>(null);
  const actionInFlightRef = useRef(new Set<string>());
  const removeTriggerRef = useRef<HTMLButtonElement | null>(null);
  const accountsHeadingRef = useRef<HTMLHeadingElement | null>(null);
  const service = connectorName(t, manifest.manifestId);
  const canConnect = manifest.supported && manifest.authMethod.includes("oauth") && Boolean(manifest.baseScopes?.length);
  const oauth = useConnectorOAuthFlow({
    manifest,
    onAct,
    onReconnectState: setActionState,
    onRefresh,
    onStarted: () => setConsentTarget(null),
  });

  async function checkConnection(account: ConnectorAccount) {
    const actionKey = `check:${account.connectorId}`;
    if (actionInFlightRef.current.has(actionKey)) return;
    actionInFlightRef.current.add(actionKey);
    setActionState({ connectorId: account.connectorId, kind: "check", state: "working", messageKey: "checking_connection" });
    try {
      const result = await integrationApi.test(account.connectorId);
      setHealth((current) => ({ ...current, [account.connectorId]: result }));
      setActionState({ connectorId: account.connectorId, kind: "check", state: "success", messageKey: "connection_checked" });
    } catch {
      setActionState({ connectorId: account.connectorId, kind: "check", state: "error", messageKey: "check_failed" });
    } finally {
      actionInFlightRef.current.delete(actionKey);
    }
  }

  async function confirmRemove() {
    if (!removeAccount) return;
    const account = removeAccount;
    const actionKey = `remove:${account.connectorId}`;
    if (actionInFlightRef.current.has(actionKey)) return;
    actionInFlightRef.current.add(actionKey);
    try {
      setActionState({ connectorId: account.connectorId, kind: "remove", state: "working", messageKey: "removing_account" });
      const succeeded = await onAct(
        () => integrationApi.disconnect(account.connectorId),
        "remove_failed",
      );
      if (succeeded) {
        setRemoveAccount(null);
        setActionState({ connectorId: account.connectorId, kind: "remove", state: "success", messageKey: "account_removed" });
        window.requestAnimationFrame(() => accountsHeadingRef.current?.focus());
      } else {
        setActionState({ connectorId: account.connectorId, kind: "remove", state: "error", messageKey: "remove_failed" });
      }
    } finally {
      actionInFlightRef.current.delete(actionKey);
    }
  }

  function cancelRemove() {
    setRemoveAccount(null);
    window.requestAnimationFrame(() => removeTriggerRef.current?.focus());
  }

  function actionFor(account: ConnectorAccount, kind: IntegrationActionKind) {
    return actionState?.connectorId === account.connectorId && actionState.kind === kind
      ? actionState
      : null;
  }

  const connectionSummaryKey = accounts.length
    ? genericConnectionSummaryKey(accounts)
    : manifest.supported
      ? "integrations.ready_to_connect"
      : "integrations.unavailable";
  const activeRemoveAction = removeAccount ? actionFor(removeAccount, "remove") : null;
  const removalErrorVisible = Boolean(removeAccount && activeRemoveAction?.state === "error");

  return <div className="mx-auto max-w-3xl">
    <GenericConnectorHeader
      accountsPresent={accounts.length > 0} actionState={actionState}
      busy={busy} canConnect={canConnect} connectionSummaryKey={connectionSummaryKey}
      error={error} manifestId={manifest.manifestId} oauthPending={oauth.pending}
      oauthStatusError={oauth.statusError} removeDialogOpen={Boolean(removeAccount)}
      onConnect={() => setConsentTarget({ scopes: manifest.baseScopes ?? [] })} onReview={() => setDetailsOpen(true)}
      service={service}
    />
    {!manifest.supported ? <ConnectorAvailabilityNotice reasonCode={manifest.availabilityReasonCode} service={service} /> : null}
    <div className="mt-7">
      <h3 className="text-sm font-semibold outline-none" ref={accountsHeadingRef} tabIndex={-1}>{t("integrations.accounts")}</h3>
      <div className="mt-3 grid gap-3">
        {accounts.length === 0
          ? <p className="rounded border border-dashed p-4 text-sm text-[var(--foreground-muted)]">{t("integrations.no_accounts")}</p>
          : accounts.map((account) => <ConnectedAccountCard
            account={account}
            busy={busy}
            checkAction={actionFor(account, "check")}
            health={health[account.connectorId]}
            key={account.connectorId}
            manifest={manifest}
            onCheck={() => void checkConnection(account)}
            onGrantOperation={(grant) => setConsentTarget({
              connectorId: account.connectorId,
              operation: grant.capabilityId,
              scopes: grant.requiredScopes,
            })}
            onReconnect={() => setConsentTarget({
              connectorId: account.connectorId,
              scopes: manifest.baseScopes ?? [],
            })}
            onRefresh={onRefresh}
            onRemove={(trigger) => {
              removeTriggerRef.current = trigger;
              setRemoveAccount(account);
            }}
            onTurnOnMessaging={onTurnOnMessaging}
            projects={projects}
            reconnectAction={actionFor(account, "reconnect")}
          />)}
      </div>
    </div>
    <details className="mt-7" onToggle={(event) => setDetailsOpen(event.currentTarget.open)} open={detailsOpen}>
      <summary className="cursor-pointer text-sm font-semibold">{t("common.details")}</summary>
      {detailsOpen ? <div className="mt-4 grid gap-5 sm:grid-cols-2">
        <Info title={t("integrations.permissions")} items={[capabilitySummary(t, manifest.manifestId)]} />
        <Info title={t("integrations.destinations")} items={manifest.dataDestinations} />
      </div> : null}
    </details>
    {consentTarget ? <GenericConsentDialog busy={busy} destinations={manifest.dataDestinations} onCancel={() => setConsentTarget(null)} onContinue={() => void oauth.start(consentTarget)} scopes={consentTarget.scopes} service={service} /> : null}
    {removeAccount ? <RemoveConnectionDialog account={removeAccount} busy={activeRemoveAction?.state === "working"} error={removalErrorVisible} onCancel={cancelRemove} onConfirm={() => void confirmRemove()} /> : null}
  </div>;
}

function GenericConnectorHeader({
  accountsPresent,
  actionState,
  busy,
  canConnect,
  connectionSummaryKey,
  error,
  manifestId,
  oauthPending,
  oauthStatusError,
  onConnect,
  onReview,
  removeDialogOpen,
  service,
}: {
  accountsPresent: boolean;
  actionState: IntegrationActionState | null;
  busy: boolean;
  canConnect: boolean;
  connectionSummaryKey: string;
  error: string;
  manifestId: string;
  oauthPending: boolean;
  oauthStatusError: string;
  onConnect: () => void;
  onReview: () => void;
  removeDialogOpen: boolean;
  service: string;
}) {
  const { t } = useI18n();
  const showAction = removeDialogOpen && actionState?.kind === "remove" ? null : actionState;
  const showError = removeDialogOpen && error === "remove_failed" ? "" : error;
  return <>
    <div className="flex items-start justify-between gap-4">
      <div>
        <h2 className="text-2xl font-semibold">{service}</h2>
        <p className="mt-2 text-sm text-[var(--foreground-muted)]">{capabilitySummary(t, manifestId)}</p>
        <p className="mt-2 text-sm">{t(connectionSummaryKey)}</p>
      </div>
      {accountsPresent
        ? <button className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]" onClick={onReview} type="button">{t("integrations.review_access")}</button>
        : canConnect
          ? <button
            aria-busy={busy || oauthPending}
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
            data-action-state={busy || oauthPending ? "working" : "idle"}
            disabled={busy || oauthPending}
            onClick={onConnect}
            type="button"
          >{t(oauthPending ? "setup.connector_connecting" : "integrations.connect")}</button>
          : null}
    </div>
    {showAction ? <p aria-live="polite" className={`mt-4 text-sm ${showAction.state === "success" ? "text-[var(--success)]" : showAction.state === "error" ? "text-[var(--destructive)]" : "text-[var(--foreground-muted)]"}`} role={showAction.state === "error" ? "alert" : "status"}>{t(`integration_actions.${showAction.messageKey}`)}</p> : null}
    {showError ? <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">{t(showError === "oauth_broker_unreachable" ? "integrations.oauth_broker.unreachable" : `integration_errors.${showError}`)}</p> : null}
    {oauthStatusError ? <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">{oauthStatusError}</p> : null}
  </>;
}

function ConnectedAccountCard({
  account,
  busy,
  checkAction,
  health,
  manifest,
  onCheck,
  onGrantOperation,
  onReconnect,
  onRefresh,
  onRemove,
  onTurnOnMessaging,
  projects,
  reconnectAction,
}: {
  account: ConnectorAccount;
  busy: boolean;
  checkAction: IntegrationActionState | null;
  health?: CapabilityHealth;
  manifest: ConnectorManifest;
  onCheck: () => void;
  onGrantOperation: (grant: ConnectorCapabilityGrant) => void;
  onReconnect: () => void;
  onRefresh: () => Promise<void>;
  onRemove: (trigger: HTMLButtonElement) => void;
  onTurnOnMessaging?: () => void;
  projects: ProjectRecord[];
  reconnectAction: IntegrationActionState | null;
}) {
  const { t } = useI18n();
  const ready = READY_CONNECTION_STATES.has(account.connectionState);
  const canReconnect = manifest.authMethod.includes("oauth");
  const slackMessagingEnabled = account.grantedScopes.includes("chat:write");
  const slackMessagingAvailable = manifest.operationGrants
    ?.find((grant) => grant.operation === "slack.messaging")?.available !== false;
  return <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <p className="text-sm font-semibold">{account.accountLabel || t("integrations.connecting")}</p>
        <p className="mt-1 text-xs text-[var(--foreground-muted)]">{connectorState(t, account.connectionState)}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        {!ready && canReconnect ? (
          <button
            aria-busy={reconnectAction?.state === "working"}
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-50"
            data-action-state={reconnectAction?.state ?? "idle"}
            disabled={busy || reconnectAction?.state === "working"}
            onClick={onReconnect}
            type="button"
          >
            {reconnectAction?.state === "working"
              ? t("integration_actions.reconnecting")
              : t("integration_actions.reconnect")}
          </button>
        ) : null}
        <button
          aria-busy={checkAction?.state === "working"}
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-xs font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
          data-action-state={checkAction?.state ?? "idle"}
          disabled={busy || checkAction?.state === "working"}
          onClick={onCheck}
          type="button"
        >
          {checkAction?.state === "working"
            ? t("integration_actions.checking")
            : t("integrations.check_connection")}
        </button>
        <button className="rounded-[var(--radius-sm)] border border-[var(--destructive)] px-3 py-2 text-xs font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:opacity-50" disabled={busy} onClick={(event) => onRemove(event.currentTarget)} type="button">{t("integration_actions.remove_connection")}</button>
      </div>
    </div>
    {manifest.manifestId === "slack" ? <div className="mt-4 flex flex-wrap items-center justify-between gap-3 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-3">
      <p className="text-sm text-[var(--foreground-muted)]">{t(slackMessagingEnabled ? "integrations.slack_messaging_on" : "integrations.slack_read_only")}</p>
      {!slackMessagingEnabled && slackMessagingAvailable && onTurnOnMessaging ? <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-semibold hover:bg-[var(--fill-hover)]" onClick={onTurnOnMessaging} type="button">{t("integrations.turn_on_messaging")}</button> : null}
    </div> : null}
    {manifest.manifestId === "google_workspace" ? <GoogleCapabilityAccess
      busy={busy}
      grants={account.capabilityGrants ?? []}
      onGrant={onGrantOperation}
    /> : null}
    {health ? <p className="mt-3 text-xs text-[var(--foreground-muted)]" role="status">{t("integration_actions.connection_result", { state: connectorState(t, health.state) })}</p> : null}
    {manifest.projectEligible ? <div className="mt-4"><ConnectorProjectScopeControls
      account={account}
      disabled={busy}
      projects={projects}
      saveScope={async (allProjectsEnabled, enabledProjectIds) => {
        const scope = await integrationApi.setProjectScope(account.connectorId, allProjectsEnabled, enabledProjectIds);
        void onRefresh().catch(() => undefined);
        return scope;
      }}
    /></div> : null}
  </section>;
}

function GoogleCapabilityAccess({
  busy,
  grants,
  onGrant,
}: {
  busy: boolean;
  grants: ConnectorCapabilityGrant[];
  onGrant: (grant: ConnectorCapabilityGrant) => void;
}) {
  const { t } = useI18n();
  return <div className="mt-4 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-3">
    <h4 className="text-sm font-semibold">{t("sprint_299.google.title")}</h4>
    <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">{t("sprint_299.google.help")}</p>
    <ul className="mt-3 grid gap-2">
      {GOOGLE_CAPABILITY_GROUPS.map((group) => {
        const grant = grants.find((item) => item.capabilityId === group.operation);
        if (!grant) return null;
        return <li className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--radius-sm)] bg-[var(--background)] px-3 py-2" key={group.operation}>
          <div>
            <p className="text-sm font-medium">{t(`sprint_299.google.${group.labelKey}`)}</p>
            <p className={`mt-0.5 text-xs ${grant.granted ? "text-[var(--success)]" : "text-[var(--foreground-muted)]"}`}>
              {t(grant.granted ? "sprint_299.google.access_allowed" : "sprint_299.google.access_not_allowed")}
            </p>
          </div>
          {!grant.granted && grant.available ? <button
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-xs font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
            disabled={busy}
            onClick={() => onGrant(grant)}
            type="button"
          >{t(busy ? "sprint_299.google.allowing_access" : "sprint_299.google.allow_access")}</button> : null}
        </li>;
      })}
    </ul>
  </div>;
}

function Info({ title, items }: { title: string; items: string[] }) {
  return <div><h3 className="text-sm font-semibold">{title}</h3><ul className="mt-2 grid gap-2 text-sm text-[var(--foreground-muted)]">{items.map((item) => <li className="rounded bg-[var(--accent-background)] p-2" key={item}>{item}</li>)}</ul></div>;
}

function RemoveConnectionDialog({ account, busy, error, onCancel, onConfirm }: { account: ConnectorAccount; busy: boolean; error: boolean; onCancel: () => void; onConfirm: () => void }) {
  const { t } = useI18n();
  const label = account.accountLabel || t("integrations.connecting");
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(true, `connector-remove-${dialogId}`);
  if (!hasDialogTurn) return null;
  return <ApprovalDialogFrame
    description={t("integrations.remove_help")}
    eyebrow={t("integration_actions.remove_connection")}
    footer={<>
      <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm transition-colors hover:bg-[var(--fill-hover)] disabled:opacity-50" data-approval-initial-focus disabled={busy} onClick={onCancel} type="button">{t("common.cancel")}</button>
      <button aria-busy={busy} className="rounded-[var(--radius-sm)] border border-[var(--destructive)] px-3 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-wait disabled:opacity-50" data-action-state={busy ? "working" : error ? "error" : "idle"} disabled={busy} onClick={onConfirm} type="button">{busy ? t("integration_actions.removing_account") : t("integration_actions.remove_connection")}</button>
    </>}
    onDismiss={() => { if (!busy) onCancel(); }}
    title={t("integration_actions.remove_title", { account: label })}
  >
    {error ? <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">{t("integration_errors.remove_failed")}</p> : null}
  </ApprovalDialogFrame>;
}

function connectorState(t: TranslateFn, value: string) {
  const state = ["authorized", "reachable", "configured", "degraded", "expired", "revoked", "offline"].includes(value) ? value : "unknown";
  return t(`integrations.states.${state}`);
}

function connectorName(t: TranslateFn, manifestId: string) {
  if (isMicrosoft365Manifest(manifestId)) return t("microsoft365.product_name");
  const id = ["google_workspace", "slack", "apple_apps", "mcp_runtime"].includes(manifestId) ? manifestId : "other";
  return t(`integrations.service_names.${id}`);
}

function capabilitySummary(t: TranslateFn, manifestId: string) {
  if (isMicrosoft365Manifest(manifestId)) return t("microsoft365.purpose");
  const id = ["google_workspace", "slack", "apple_apps", "mcp_runtime"].includes(manifestId) ? manifestId : "other";
  return t(`integrations.capability_summaries.${id}`);
}

function GenericConsentDialog({ busy, destinations, onCancel, onContinue, scopes, service }: { busy: boolean; destinations: string[]; onCancel: () => void; onContinue: () => void; scopes: string[]; service: string }) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(true, `connector-consent-${dialogId}`);
  const destinationLabels = genericDestinationLabels(t, destinations);
  const scopeLabels = [...new Set(scopes.map((scope) => genericScopeLabel(t, scope)))];
  if (!hasDialogTurn) return null;
  return <ApprovalDialogFrame
    description={t("integrations.consent_help")}
    eyebrow={t("integrations.review_access")}
    footer={<>
      <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50" data-approval-initial-focus disabled={busy} onClick={onCancel} type="button">{t("common.cancel")}</button>
      <button aria-busy={busy} className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50" data-action-state={busy ? "working" : "idle"} disabled={busy} onClick={onContinue} type="button">{busy ? t("integration_actions.opening_service") : t("integrations.continue_to_service", { service })}</button>
    </>}
    onDismiss={() => { if (!busy) onCancel(); }}
    title={t("integrations.consent_title", { service })}
  >
    <details className="group mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)]">
      <summary className="cursor-pointer list-none px-3 py-2.5 text-sm font-semibold [&::-webkit-details-marker]:hidden">{t("common.details")}</summary>
      <div className="space-y-5 border-t border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wide">{t("integrations.exact_access")}</h3>
          <ul className="mt-2 grid gap-1">{scopeLabels.map((label) => <li className="rounded bg-[var(--background)] p-2 text-sm" key={label}>{label}</li>)}</ul>
        </div>
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wide">{t("integrations.destinations")}</h3>
          <ul className="mt-2 grid gap-1">{destinationLabels.map((destination) => <li className="rounded bg-[var(--background)] p-2 text-sm" key={destination}>{destination}</li>)}</ul>
        </div>
      </div>
    </details>
  </ApprovalDialogFrame>;
}
