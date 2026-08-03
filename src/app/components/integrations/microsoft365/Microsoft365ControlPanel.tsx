"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useApprovalDialogTurn } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import type { ProjectRecord } from "../../projects/projectClient";
import { ConnectorAvailabilityNotice } from "../ConnectorAvailabilityNotice";
import { ConnectorProjectScopeControls } from "../ConnectorProjectScopeControls";
import type { ConnectorManifest } from "../integrationClient";
import {
  microsoft365Api,
  type Microsoft365Account,
  type Microsoft365CapabilityGrant,
  type Microsoft365Health,
} from "./microsoft365Client";

type Props = { manifest: ConnectorManifest; projects: ProjectRecord[] };
type ConsentReview = {
  kind: "base" | "reconnect" | "capability" | "inspect";
  connectorId?: string;
  operation?: string;
  scopes: string[];
};

export function Microsoft365ControlPanel({ manifest, projects }: Props) {
  const { t } = useI18n();
  const [accounts, setAccounts] = useState<Microsoft365Account[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [health, setHealth] = useState<Microsoft365Health | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [finishing, setFinishing] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [disconnectReviewOpen, setDisconnectReviewOpen] = useState(false);
  const [error, setError] = useState("");
  const [consentReview, setConsentReview] = useState<ConsentReview | null>(null);
  const pollGeneration = useRef(0);
  const operationInFlight = useRef(false);

  const selected = useMemo(
    () => accounts.find((account) => account.connectorId === selectedId) ?? null,
    [accounts, selectedId],
  );
  const capabilityGrants = effectiveCapabilityGrants(selected, manifest);

  const applyAccounts = useCallback((next: Microsoft365Account[]) => {
    setAccounts(next);
    setSelectedId((current) => next.some((account) => account.connectorId === current)
      ? current
      : (next[0]?.connectorId ?? ""));
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try { applyAccounts(await microsoft365Api.accounts()); setError(""); }
    catch { applyAccounts([]); setError("unavailable"); }
    finally { setLoading(false); }
  }, [applyAccounts]);

  useEffect(() => {
    const timer = window.setTimeout(() => { void load(); }, 0);
    return () => {
      window.clearTimeout(timer);
      pollGeneration.current += 1;
    };
  }, [load]);

  async function waitForOauth(connectorId: string, startedAtMs: number, previousProbeAtMs?: number | null) {
    const generation = ++pollGeneration.current;
    setFinishing(true);
    try {
      for (let attempt = 0; attempt < 30; attempt += 1) {
        const next = await microsoft365Api.accounts();
        if (generation !== pollGeneration.current) return;
        applyAccounts(next);
        const account = next.find((item) => item.connectorId === connectorId);
        const probeAt = account?.lastProbeAtMs;
        const freshProbe = typeof probeAt === "number" && probeAt >= startedAtMs && (typeof previousProbeAtMs !== "number" || probeAt > previousProbeAtMs);
        if (account && freshProbe) {
          setError(account.lastProbeCode === "oauth_completed" ? "" : "connection_attention");
          return;
        }
        await new Promise((resolve) => window.setTimeout(resolve, 1_000));
      }
      if (generation === pollGeneration.current) setError("connection_timeout");
    } catch {
      if (generation === pollGeneration.current) setError("connection_status_failed");
    } finally {
      if (generation === pollGeneration.current) setFinishing(false);
    }
  }

  async function continueConsent(review: ConsentReview) {
    if (review.kind === "inspect") { setConsentReview(null); return; }
    if (operationInFlight.current) return;
    operationInFlight.current = true;
    setBusy(true); setError("");
    try {
      const startedAtMs = Date.now();
      const previousProbeAtMs = review.connectorId
        ? accounts.find((account) => account.connectorId === review.connectorId)?.lastProbeAtMs
        : undefined;
      const attempt = await microsoft365Api.beginOauth(manifest.manifestId, {
        ...(review.connectorId ? { connectorId: review.connectorId } : {}),
        ...(review.operation ? { requestedOperations: [review.operation] } : {}),
      });
      setConsentReview(null);
      await waitForOauth(attempt.connectorId, startedAtMs, previousProbeAtMs);
    } catch { setError("action_failed"); }
    finally { operationInFlight.current = false; setBusy(false); }
  }

  async function disconnectSelected() {
    if (!selected || operationInFlight.current) return;
    operationInFlight.current = true;
    setBusy(true); setError("");
    try {
      await microsoft365Api.disconnect(selected.connectorId);
      setDisconnectReviewOpen(false);
      setHealth(null);
      setDetailsOpen(false);
      await load();
    } catch { setError("disconnect_failed"); }
    finally { operationInFlight.current = false; setBusy(false); }
  }

  if (loading) return <p className="mt-7 text-sm text-[var(--foreground-muted)]">{t("microsoft365.loading")}</p>;
  if (!accounts.length) return <EmptyMicrosoftAccountPanel busy={busy} consentReview={consentReview} error={error} finishing={finishing} manifest={manifest} onConnect={() => setConsentReview({ kind: "base", scopes: manifest.baseScopes ?? [] })} onContinue={continueConsent} onDismiss={() => setConsentReview(null)} onRetry={load} />;
  if (!selected) return null;
  return <ConnectedMicrosoftAccountPanel
    accounts={accounts} busy={busy} capabilityGrants={capabilityGrants} consentReview={consentReview}
    detailsOpen={detailsOpen} disconnectReviewOpen={disconnectReviewOpen} error={error} finishing={finishing}
    health={health} manifest={manifest} onConsent={setConsentReview} onContinue={continueConsent}
    onDetails={setDetailsOpen} onDisconnect={() => void disconnectSelected()} onDisconnectReview={setDisconnectReviewOpen} onRefresh={() => void load()}
    onScopeSaved={() => { void load().catch(() => undefined); }}
    onSelect={(connectorId) => { setSelectedId(connectorId); setHealth(null); setDetailsOpen(false); setDisconnectReviewOpen(false); }}
    onTest={() => void microsoft365Api.test(selected.connectorId).then(setHealth).catch(() => setError("action_failed"))}
    projects={projects} selected={selected}
  />;
}

function effectiveCapabilityGrants(
  selected: Microsoft365Account | null,
  manifest: ConnectorManifest,
): Microsoft365CapabilityGrant[] {
  if (selected?.capabilityGrants?.length) return selected.capabilityGrants;
  return (manifest.operationGrants ?? []).map((grant) => ({
    capabilityId: grant.operation,
    accessLevel: grant.accessLevel,
    requiredScopes: grant.requiredScopes,
    granted: false,
    adminConsentRequired: grant.adminConsentRequired,
    remoteMutation: grant.remoteMutation,
    available: true,
  }));
}

type EmptyPanelProps = {
  busy: boolean;
  consentReview: ConsentReview | null;
  error: string;
  finishing: boolean;
  manifest: ConnectorManifest;
  onConnect: () => void;
  onContinue: (review: ConsentReview) => Promise<void>;
  onDismiss: () => void;
  onRetry: () => Promise<void>;
};

function EmptyMicrosoftAccountPanel(props: EmptyPanelProps) {
  const { t } = useI18n();
  const canConnect = props.manifest.supported && !props.error && Boolean(props.manifest.baseScopes?.length);
  return <section className="mt-7 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-5">
    <h3 className="text-base font-semibold">{t("microsoft365.product_name")}</h3>
    <p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("microsoft365.purpose")}</p>
    {!props.manifest.supported ? <ConnectorAvailabilityNotice reasonCode={props.manifest.availabilityReasonCode} service={t("microsoft365.product_name")} /> : <>
      <p className="mt-4 text-sm" role="status">{props.finishing ? t("microsoft365.finishing") : props.error ? t("microsoft365.unavailable") : t("microsoft365.no_accounts")}</p>
      {props.error ? <button className="mt-4 rounded bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)]" onClick={() => void props.onRetry()} type="button">{t("microsoft365_labels.try_again")}</button> : null}
      {canConnect ? <button className="mt-4 rounded bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50" disabled={props.busy || props.finishing} onClick={props.onConnect} type="button">{t("microsoft365.connect")}</button> : null}
      {!props.error && !props.manifest.baseScopes?.length ? <p className="mt-3 text-xs text-[var(--warning)]">{t("microsoft365.consent_metadata_unavailable")}</p> : null}
    </>}
    {props.consentReview ? <ConsentReviewDialog busy={props.busy} destinations={props.manifest.dataDestinations} onCancel={props.onDismiss} onContinue={() => void props.onContinue(props.consentReview!)} review={props.consentReview} /> : null}
  </section>;
}

type ConnectedPanelProps = {
  accounts: Microsoft365Account[];
  busy: boolean;
  capabilityGrants: Microsoft365CapabilityGrant[];
  consentReview: ConsentReview | null;
  detailsOpen: boolean;
  disconnectReviewOpen: boolean;
  error: string;
  finishing: boolean;
  health: Microsoft365Health | null;
  manifest: ConnectorManifest;
  onConsent: (review: ConsentReview | null) => void;
  onContinue: (review: ConsentReview) => Promise<void>;
  onDetails: (open: boolean) => void;
  onDisconnect: () => void;
  onDisconnectReview: (open: boolean) => void;
  onRefresh: () => void;
  onScopeSaved: () => void;
  onSelect: (connectorId: string) => void;
  onTest: () => void;
  projects: ProjectRecord[];
  selected: Microsoft365Account;
};

function ConnectedMicrosoftAccountPanel(props: ConnectedPanelProps) {
  const { t } = useI18n();
  const repair = needsRepair(props.selected.connectionState);
  const requestReconnect = () => props.onConsent({
    kind: "reconnect",
    connectorId: props.selected.connectorId,
    scopes: exactOauthScopes(props.manifest.baseScopes ?? [], props.selected.grantedScopes),
  });
  return <section className="mt-7 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-5">
    <div><h3 className="text-base font-semibold">{t("microsoft365.product_name")}</h3><p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("microsoft365.purpose")}</p></div>
    <AccountPicker accounts={props.accounts} onSelect={props.onSelect} selected={props.selected} />
    <div className="mt-5 grid gap-5">
      <div className="rounded bg-[var(--accent-background)] p-3 text-sm" role="status"><p className="font-medium">{props.finishing ? t("microsoft365.finishing") : readinessLabel(t, props.selected.connectionState)}</p></div>
      <ConnectorProjectScopeControls account={props.selected} disabled={props.busy || props.finishing} projects={props.projects} saveScope={async (allProjectsEnabled, enabledProjectIds) => {
        const scope = await microsoft365Api.setProjectScope(props.selected.connectorId, allProjectsEnabled, enabledProjectIds);
        props.onScopeSaved();
        return scope;
      }} />
      <button className="w-fit rounded bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50" disabled={props.busy || props.finishing} onClick={repair ? requestReconnect : () => props.onDetails(true)} type="button">{repair ? t("microsoft365.fix_connection") : t("microsoft365.manage_access")}</button>
      <AccountAccessDetails {...props} onReconnect={requestReconnect} />
      {props.error ? <p className="text-sm text-[var(--warning)]" role="alert">{t(`microsoft365.errors.${knownError(props.error)}`)}</p> : null}
      {props.consentReview ? <ConsentReviewDialog busy={props.busy} destinations={props.manifest.dataDestinations} onCancel={() => props.onConsent(null)} onContinue={() => void props.onContinue(props.consentReview!)} review={props.consentReview} /> : null}
      {props.disconnectReviewOpen ? <DisconnectReviewDialog accountLabel={props.selected.accountLabel} busy={props.busy} onCancel={() => props.onDisconnectReview(false)} onConfirm={props.onDisconnect} /> : null}
    </div>
  </section>;
}

function AccountPicker({ accounts, onSelect, selected }: { accounts: Microsoft365Account[]; onSelect: (connectorId: string) => void; selected: Microsoft365Account }) {
  const { t } = useI18n();
  return <div className="mt-4 grid gap-2 sm:grid-cols-2" role="radiogroup" aria-label={t("microsoft365.switch_account")}>{accounts.map((account) => <button aria-checked={selected.connectorId === account.connectorId} className={`rounded-[var(--radius-sm)] border p-3 text-left ${selected.connectorId === account.connectorId ? "border-[var(--foreground)] bg-[var(--fill-selected)]" : "border-[var(--border-soft)]"}`} key={account.connectorId} onClick={() => onSelect(account.connectorId)} role="radio" type="button"><span className="block text-sm font-semibold">{account.accountLabel}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{stateLabel(t, account.connectionState)}</span></button>)}</div>;
}

function AccountAccessDetails(props: ConnectedPanelProps & { onReconnect: () => void }) {
  const { t } = useI18n();
  const selected = props.selected;
  return <details onToggle={(event) => props.onDetails(event.currentTarget.open)} open={props.detailsOpen}>
    <summary className="cursor-pointer text-sm font-semibold">{t("common.details")}</summary>
    <div className="mt-4 grid gap-5">
      <AccountIdentity account={selected} />
      <CapabilityAccess busy={props.busy} grants={props.capabilityGrants} onRequest={(grant) => props.onConsent({ kind: "capability", connectorId: selected.connectorId, operation: grant.capabilityId, scopes: exactOauthScopes(props.manifest.baseScopes ?? [], selected.grantedScopes, grant.requiredScopes) })} />
      <DataRouting account={selected} />
      <div className="flex flex-wrap gap-2">
        <button className="rounded border px-3 py-2 text-xs font-semibold" disabled={props.busy} onClick={props.onReconnect} type="button">{t("microsoft365.reconnect")}</button>
        <button className="rounded border px-3 py-2 text-xs font-semibold" disabled={props.busy} onClick={() => props.onConsent({ kind: "inspect", connectorId: selected.connectorId, scopes: selected.grantedScopes })} type="button">{t("microsoft365.review_consent")}</button>
        <button className="rounded border px-3 py-2 text-xs font-semibold" disabled={props.busy} onClick={props.onTest} type="button">{t("microsoft365.check_health")}</button>
        <button className="rounded border px-3 py-2 text-xs font-semibold" disabled={props.busy} onClick={props.onRefresh} type="button">{t("common.refresh")}</button>
        <button className="rounded border border-[var(--destructive)] px-3 py-2 text-xs font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:opacity-50" disabled={props.busy} onClick={() => props.onDisconnectReview(true)} type="button">{t("microsoft365.remove_account")}</button>
      </div>
      {props.health ? <div className="rounded bg-[var(--accent-background)] p-3 text-sm" role="status"><p className="font-semibold">{stateLabel(t, props.health.state)}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{healthDescription(t, props.health.detailCode)}</p>{props.health.repairActionCode ? <p className="mt-1 text-xs">{healthRepair(t, props.health.repairActionCode)}</p> : null}</div> : null}
    </div>
  </details>;
}

function AccountIdentity({ account }: { account: Microsoft365Account }) {
  const { t } = useI18n();
  const organization = account.tenantLabel || (["work", "school"].includes(account.accountKind ?? "") ? t("microsoft365_labels.work_school_organization") : t("microsoft365.tenant_unknown"));
  const tenantId = account.tenantId && /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(account.tenantId) ? account.tenantId : t("microsoft365.not_reported");
  return <div><h4 className="text-sm font-semibold">{t("microsoft365.account_details")}</h4><dl className="mt-2 grid grid-cols-[8rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs"><dt className="text-[var(--foreground-muted)]">{t("microsoft365.account")}</dt><dd>{account.accountPrincipal || account.accountLabel}</dd><dt className="text-[var(--foreground-muted)]">{t("microsoft365.tenant")}</dt><dd>{organization}</dd><dt className="text-[var(--foreground-muted)]">{t("microsoft365_labels.tenant_id")}</dt><dd className="truncate">{tenantId}</dd><dt className="text-[var(--foreground-muted)]">{t("microsoft365.account_type")}</dt><dd>{account.accountKind ? accountKindLabel(t, account.accountKind) : t("common.unknown")}</dd><dt className="text-[var(--foreground-muted)]">{t("microsoft365.identity_binding")}</dt><dd className="truncate">{account.identityBindingHash || t("microsoft365.not_reported")}</dd></dl></div>;
}

function CapabilityAccess({ busy, grants, onRequest }: { busy: boolean; grants: Microsoft365CapabilityGrant[]; onRequest: (grant: Microsoft365CapabilityGrant) => void }) {
  const { t } = useI18n();
  return <div><h4 className="text-sm font-semibold">{t("microsoft365.available_actions")}</h4><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("microsoft365.available_actions_help")}</p>{grants.length ? <ul className="mt-2 grid gap-2 sm:grid-cols-2">{grants.map((grant) => <li className="rounded bg-[var(--accent-background)] p-2 text-xs" key={grant.capabilityId}><span className="font-semibold">{capabilityLabel(t, grant.capabilityId)}</span><span className="mt-1 block">{!grant.available ? t("microsoft365.work_account_required") : grant.granted ? t("microsoft365.ready_to_use") : t("microsoft365.access_needed")}{grant.available && grant.remoteMutation ? ` · ${t("microsoft365.approval_required")}` : ""}</span>{grant.available && grant.requiredScopes.length ? <button className="mt-2 rounded border px-2 py-1 font-semibold disabled:opacity-50" disabled={busy} onClick={() => onRequest(grant)} type="button">{grant.granted ? t("microsoft365.reauthorize_capability") : t("microsoft365.grant_capability")}</button> : grant.available ? <span className="mt-2 block text-[var(--foreground-muted)]">{t("microsoft365.no_remote_scope")}</span> : null}</li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("microsoft365.actions_unavailable")}</p>}</div>;
}

function DataRouting({ account }: { account: Microsoft365Account }) {
  const { t } = useI18n(); const routing = account.dataRouting ?? [];
  return <div><h4 className="text-sm font-semibold">{t("microsoft365.where_data_goes")}</h4>{routing.length ? <ul className="mt-2 grid gap-1 text-xs">{routing.map((route) => <li className="rounded bg-[var(--accent-background)] p-2" key={route}>{routingLabel(t, route)}</li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("microsoft365.routes_unavailable")}</p>}<p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("microsoft365.routing_project_controlled")}</p></div>;
}

function ConsentReviewDialog({ busy, destinations, onCancel, onContinue, review }: { busy: boolean; destinations: string[]; onCancel: () => void; onContinue: () => void; review: ConsentReview }) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(true, `microsoft-consent-${dialogId}`);
  const purposes = scopePurposes(review.scopes, t);
  const routes = destinationPurposes(destinations, t);
  if (!hasDialogTurn) return null;
  return <ApprovalDialogFrame
    description={review.kind === "inspect" ? t("microsoft365.current_consent_help") : t("microsoft365.consent_review_help")}
    eyebrow={t("microsoft365.product_name")}
    footer={<>
      <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50" data-approval-initial-focus disabled={busy} onClick={onCancel} type="button">{review.kind === "inspect" ? t("common.close") : t("common.cancel")}</button>
      {review.kind !== "inspect" ? <button aria-busy={busy} className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50" disabled={busy} onClick={onContinue} type="button">{t("microsoft365.continue_to_microsoft")}</button> : null}
    </>}
    onDismiss={() => { if (!busy) onCancel(); }}
    title={review.kind === "inspect" ? t("microsoft365.current_consent") : t("microsoft365.consent_review_title")}
  >
    {review.operation ? <p className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-3 text-sm font-medium">{capabilityLabel(t, review.operation)}</p> : null}
    <details className="group mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)]">
      <summary className="cursor-pointer list-none px-3 py-2.5 text-sm font-semibold [&::-webkit-details-marker]:hidden">{t("common.details")}</summary>
      <div className="space-y-5 border-t border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wide">{t("microsoft365_labels.what_this_allows")}</h3>
          {purposes.length ? <ul className="mt-2 grid gap-1">{purposes.map((purpose) => <li className="rounded bg-[var(--background)] p-2 text-sm" key={purpose}>{purpose}</li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("microsoft365.no_remote_scope")}</p>}
        </div>
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wide">{t("microsoft365_labels.connects_to")}</h3>
          {routes.length ? <ul className="mt-2 grid gap-1">{routes.map((route) => <li className="rounded bg-[var(--background)] p-2 text-sm" key={route}>{route}</li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("microsoft365.no_destination_reported")}</p>}
        </div>
      </div>
    </details>
    <p className="mt-4 text-xs text-[var(--foreground-muted)]">{t("microsoft365.consent_not_approval")}</p>
  </ApprovalDialogFrame>;
}

const SCOPE_PURPOSE_KEYS: Record<string, string> = {
  openid: "identify_account", profile: "identify_account", email: "identify_account", "User.Read": "identify_account",
  offline_access: "keep_connected", "Mail.Read": "read_mail", "Mail.ReadWrite": "prepare_mail",
  "Calendars.Read": "read_calendar", "Calendars.ReadWrite": "prepare_calendar",
  "Files.Read": "read_onedrive", "Files.ReadWrite": "update_onedrive",
  "Sites.Read.All": "read_sharepoint", "Sites.ReadWrite.All": "update_sharepoint",
  "Chat.Read": "read_teams", "Chat.ReadWrite": "prepare_teams",
};

function scopePurposes(scopes: string[], t: (key: string) => string) {
  return [...new Set(scopes.map((scope) => t(`microsoft365_labels.scope_${SCOPE_PURPOSE_KEYS[scope] ?? "other"}`)))];
}

function destinationPurposes(destinations: string[], t: (key: string) => string) {
  return [...new Set(destinations.map((destination) => t(destination.includes("login.microsoftonline.com")
    ? "microsoft365_labels.destination_sign_in"
    : /(?:graph\.microsoft\.com|sharepoint|1drv\.com)/.test(destination)
      ? "microsoft365_labels.destination_services"
      : "microsoft365_labels.destination_other")))];
}

function DisconnectReviewDialog({ accountLabel, busy, onCancel, onConfirm }: { accountLabel: string; busy: boolean; onCancel: () => void; onConfirm: () => void }) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(true, `microsoft-disconnect-${dialogId}`);
  if (!hasDialogTurn) return null;
  return <ApprovalDialogFrame
    description={t("microsoft365.remove_account_help")}
    eyebrow={t("microsoft365.product_name")}
    footer={<>
      <button className="rounded border px-3 py-2 text-sm disabled:opacity-50" data-approval-initial-focus disabled={busy} onClick={onCancel} type="button">{t("common.cancel")}</button>
      <button aria-busy={busy} className="rounded border border-[var(--destructive)] px-3 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-wait disabled:opacity-50" data-action-state={busy ? "working" : "idle"} disabled={busy} onClick={onConfirm} type="button">{t("microsoft365.remove_account_confirm")}</button>
    </>}
    onDismiss={() => { if (!busy) onCancel(); }}
    title={t("microsoft365.remove_account_title")}
  >
    <p className="mt-5 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-3 text-sm font-medium">{accountLabel}</p>
  </ApprovalDialogFrame>;
}

const KNOWN_STATES = new Set(["healthy", "reachable", "configured", "blocked", "unsupported", "disconnected", "degraded", "partial", "offline", "expired", "revoked", "rate_limited", "tenant_policy", "unavailable", "authorized"]);
const KNOWN_ACCOUNTS = new Set(["personal", "work", "school"]);
const KNOWN_CAPABILITIES = new Set(["outlook.mail.search", "outlook.mail.read", "outlook.mail.draft", "outlook.calendar.read", "outlook.calendar.draft_event", "onedrive.file.search", "onedrive.file.read", "onedrive.file.write", "sharepoint.file.search", "sharepoint.file.read", "sharepoint.file.write", "teams.chat.search", "teams.chat.draft_message"]);
const KNOWN_ROUTING = new Set(["local_only", "microsoft_graph", "microsoft_service", "configured_cloud", "project_policy"]);
const KNOWN_DIAGNOSTICS = new Set(["connector_identity_probe_ok", "microsoft_authorization_revoked", "microsoft_rate_limited", "microsoft_tenant_policy_blocked", "microsoft_identity_offline", "microsoft_refresh_offline", "microsoft_request_offline", "microsoft_service_unavailable", "microsoft_request_rejected", "microsoft_refresh_token_missing", "unknown"]);
const KNOWN_REPAIRS = new Set(["connector_reconnect", "connector_retry_later", "connector_tenant_admin_review", "connector_check_network", "connector_test_or_reconnect", "none"]);
const KNOWN_ERRORS = new Set(["unavailable", "action_failed", "disconnect_failed", "connection_attention", "connection_timeout", "connection_status_failed"]);

function stateLabel(t: (key: string) => string, state: string) { const normalized = state.toLowerCase().replaceAll("-", "_"); return t(`microsoft365.states.${KNOWN_STATES.has(normalized) ? normalized : "unavailable"}`); }
function readinessLabel(t: (key: string) => string, state: string) { const normalized = state.toLowerCase().replaceAll("-", "_"); return t(`microsoft365.readiness.${KNOWN_STATES.has(normalized) ? normalized : "unavailable"}`); }
function needsRepair(state: string) { return !["authorized", "reachable", "healthy"].includes(state.toLowerCase()); }
function knownError(value: string) { return KNOWN_ERRORS.has(value) ? value : "action_failed"; }
function exactOauthScopes(...groups: string[][]) { return [...new Set(groups.flat().filter(Boolean))].sort((left, right) => left.localeCompare(right)); }
function accountKindLabel(t: (key: string) => string, value: string) { return t(`microsoft365.account_types.${KNOWN_ACCOUNTS.has(value) ? value : "unknown"}`); }
function capabilityLabel(t: (key: string) => string, value: string) { return t(`microsoft365.capabilities.${KNOWN_CAPABILITIES.has(value) ? value.replaceAll(".", "_") : "unknown"}`); }
function routingLabel(t: (key: string) => string, value: string) { return /^https:\/\//.test(value) ? value : t(`microsoft365.routing_values.${KNOWN_ROUTING.has(value) ? value : "unknown"}`); }
function healthDescription(t: (key: string) => string, value?: string) { return t(`microsoft365.health.${value && KNOWN_DIAGNOSTICS.has(value) ? value : "unknown"}`); }
function healthRepair(t: (key: string) => string, value?: string) { return t(`microsoft365.repairs.${value && KNOWN_REPAIRS.has(value) ? value : "none"}`); }
