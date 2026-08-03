"use client";

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import {
  DEFAULT_TRUST_PATH,
  secondaryButtonClass,
  TrustFolderPolicyForm,
  TRUST_TOOL_CATEGORIES,
  type TrustPermissionLevel,
  type TrustToolCategory,
} from "./TrustFolderPolicyForm";
import {
  formatTrustAction,
  formatTrustScopeKind,
  formatTrustStatus,
  isUntilRevokedReviewedScope,
  type TrustTranslate,
} from "./trustPresentation";

export {
  formatTrustAction,
  formatTrustScopeKind,
  formatTrustStatus,
  isUntilRevokedReviewedScope,
} from "./trustPresentation";

type SovereignTrustPolicyRecord = {
  id: number;
  directoryPath: string;
  canonicalDirectoryPath: string;
  allowedToolCategories: string[];
  permissionLevel: string;
  expiresAtMs: number | null;
  dailyTokenCostLimit: number;
  dailyCpuSecondsLimit: number;
  estimatedTokenCostReservedToday: number;
  cpuSecondsReservedToday: number;
  usageDay: number;
  createdAtMs: number;
  updatedAtMs: number;
  lastUsedAtMs: number | null;
  active: boolean;
};

type SovereignTrustSessionRecord = {
  id: string;
  sessionId: string;
  policyId: number | null;
  directoryPath: string;
  canonicalDirectoryPath: string;
  allowedToolCategories: string[];
  permissionLevel: string;
  expiresAtMs: number;
  dailyTokenCostLimit: number;
  dailyCpuSecondsLimit: number;
  estimatedTokenCostReservedToday: number;
  cpuSecondsReservedToday: number;
  usageDay: number;
  createdAtMs: number;
  lastUsedAtMs: number | null;
  active: boolean;
};

type SovereignTrustAuditEvent = {
  id: number;
  planId: string;
  operation: string;
  inputKind: string | null;
  targetPath: string | null;
  status: string;
  authorizationMode: string;
  trustTier: string | null;
  executionHash: string;
  summary: string;
  claims: string[];
  createdAtMs: number;
};

type SovereignTrustDashboardResponse = {
  policies: SovereignTrustPolicyRecord[];
  activeSessions: SovereignTrustSessionRecord[];
  auditEvents: SovereignTrustAuditEvent[];
};

type SovereignTrustMutationResponse = {
  affectedRows: number;
  message: string;
};

type ReviewedApprovalScope = { grantId: string; scopeKind: string; principal: string; projectId: string | null; taskRunId: string | null; actionClass: string; canonicalResource: string; argumentClass: string; expiresAtMs: number; maxUses: number; usedCount: number; reviewedAtMs: number; revokedAtMs: number | null; active: boolean };
type ReviewedApprovalDashboard = { grants: ReviewedApprovalScope[]; auditEvents: Array<{ id: number; eventType: string; actionClass: string; createdAtMs: number }> };
type SessionScopeTrustGrant = { grantId: string; principal: string; canonicalResource: string; actionClass: string; grantedAtMs: number };

function errorMessage(error: unknown, fallback: string) {
  return error && typeof error === "object" && "message" in error
    ? String(error.message)
    : fallback;
}

function formatTrustTier(value: string | null | undefined, t: TrustTranslate) {
  switch (value) {
    case "global_trust":
      return t("settings.privacy.trust.tier_global");
    case "session_gated":
      return t("settings.privacy.trust.tier_session");
    case "one_time":
      return t("settings.privacy.trust.tier_one_time");
    default:
      return value
        ? t("settings.privacy.trust.tier_other")
        : t("settings.privacy.trust.tier_one_time");
  }
}

function formatAuthorizationMode(mode: string, trustTier: string | null, t: TrustTranslate) {
  switch (mode) {
    case "global_trust_auto":
      return t("settings.privacy.trust.auth_global_auto");
    case "session_gated_auto":
      return t("settings.privacy.trust.auth_session_auto");
    case "trusted_auto":
      return t("settings.privacy.trust.auth_trusted_auto", {
        tier: formatTrustTier(trustTier, t),
      });
    case "manual_popup":
      return t("settings.privacy.trust.auth_manual_popup");
    default:
      return t("settings.privacy.trust.auth_recorded");
  }
}

function formatToolCategories(categories: string[], t: TrustTranslate) {
  if (categories.length === 0) return t("settings.privacy.trust.none");
  return categories
    .map((category) => {
      if (category === "shell_commands") return t("settings.privacy.trust.tool_shell");
      if (category === "external_writes") return t("settings.privacy.trust.tool_writes");
      return t("settings.privacy.trust.tool_other");
    })
    .join(", ");
}

function formatTimestamp(value: number | null | undefined, t: TrustTranslate) {
  if (!value) return t("settings.privacy.trust.never");
  return new Date(value).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function formatUsage(value: number, maximum: number, unit: string) {
  const maximumLabel = Number.isInteger(maximum)
    ? maximum.toLocaleString()
    : maximum.toFixed(1);
  const valueLabel = Number.isInteger(value) ? value.toLocaleString() : value.toFixed(2);
  return `${valueLabel}/${maximumLabel} ${unit}`;
}

function shortHash(hash: string) {
  if (hash.length <= 20) return hash;
  return `${hash.slice(0, 12)}...${hash.slice(-8)}`;
}

export function TrustPanel() {
  const { language, t } = useI18n();
  const componentSessionId = useId().replaceAll(":", "");
  const authoritySessionId = `trust-settings-${componentSessionId}`;
  const mountedRef = useRef(true);
  const [dashboard, setDashboard] =
    useState<SovereignTrustDashboardResponse | null>(null);
  const [reviewedDashboard, setReviewedDashboard] = useState<ReviewedApprovalDashboard | null>(null);
  const [sessionScopeGrants, setSessionScopeGrants] = useState<SessionScopeTrustGrant[]>([]);
  const [trustDirectoryPath, setTrustDirectoryPath] = useState(DEFAULT_TRUST_PATH);
  const [trustTier, setTrustTier] =
    useState<TrustPermissionLevel>("global_trust");
  const [categorySelection, setCategorySelection] = useState<
    Record<TrustToolCategory, boolean>
  >({
    shell_commands: true,
    external_writes: true,
  });
  const [isLoadingTrust, setIsLoadingTrust] = useState(true);
  const [isSavingTrust, setIsSavingTrust] = useState(false);
  const [isBrowsingTrustPath, setIsBrowsingTrustPath] = useState(false);
  const [browseTrustState, setBrowseTrustState] = useState<
    "idle" | "working" | "success" | "error"
  >("idle");
  const [saveTrustState, setSaveTrustState] = useState<
    "idle" | "working" | "success" | "error"
  >("idle");
  const [mutatingKey, setMutatingKey] = useState("");
  const [trustMessage, setTrustMessage] = useState("");
  const [trustLoadError, setTrustLoadError] = useState("");
  const [trustError, setTrustError] = useState("");
  const dashboardLoadGenerationRef = useRef(0);
  const dashboardLoadsInFlightRef = useRef(0);
  const visibleDashboardLoadsRef = useRef(0);

  const loadTrustDashboard = useCallback(async (
    silent = false,
    force = false,
  ) => {
    if (!force && silent && dashboardLoadsInFlightRef.current > 0) return;
    const generation = ++dashboardLoadGenerationRef.current;
    dashboardLoadsInFlightRef.current += 1;
    if (!silent) {
      visibleDashboardLoadsRef.current += 1;
      setIsLoadingTrust(true);
      setTrustLoadError("");
    }

    try {
      const [data, reviewed, sessionGrants] = await Promise.all([
        invoke<SovereignTrustDashboardResponse>("get_sovereign_trust_dashboard", { auditLimit: 40 }),
        invoke<ReviewedApprovalDashboard>("get_reviewed_approval_scopes", { filter: { projectId: null, taskRunId: null } }),
        invoke<SessionScopeTrustGrant[]>("list_session_scope_trust_grants").catch(() => []),
      ]);
      if (
        !mountedRef.current ||
        generation !== dashboardLoadGenerationRef.current
      ) return;
      setDashboard(data);
      setReviewedDashboard(reviewed);
      setSessionScopeGrants(Array.isArray(sessionGrants) ? sessionGrants : []);
    } catch (error) {
      if (
        !mountedRef.current ||
        silent ||
        generation !== dashboardLoadGenerationRef.current
      ) return;
      setTrustLoadError(errorMessage(error, t("settings.privacy.trust.load_error")));
    } finally {
      dashboardLoadsInFlightRef.current = Math.max(
        0,
        dashboardLoadsInFlightRef.current - 1,
      );
      if (!silent) {
        visibleDashboardLoadsRef.current = Math.max(
          0,
          visibleDashboardLoadsRef.current - 1,
        );
      }
      if (mountedRef.current && visibleDashboardLoadsRef.current === 0) {
        setIsLoadingTrust(false);
      }
    }
  }, [t]);

  useEffect(() => {
    mountedRef.current = true;
    const initialLoad = window.setTimeout(() => {
      void loadTrustDashboard(false, true);
    }, 0);
    const interval = window.setInterval(() => {
      void loadTrustDashboard(true);
    }, 5000);

    return () => {
      mountedRef.current = false;
      window.clearTimeout(initialLoad);
      window.clearInterval(interval);
    };
  }, [loadTrustDashboard]);

  const selectedCategories = TRUST_TOOL_CATEGORIES.filter(
    (category) => categorySelection[category.id],
  ).map((category) => category.id);
  const policies = dashboard?.policies ?? [];
  const activeSessions = dashboard?.activeSessions ?? [];
  const auditEvents = dashboard?.auditEvents ?? [];
  const reviewedScopes = reviewedDashboard?.grants ?? [];

  const browseTrustFolder = async () => {
    setIsBrowsingTrustPath(true);
    setBrowseTrustState("working");
    setSaveTrustState("idle");
    setTrustMessage("");
    setTrustError("");
    try {
      const selectedPath = await invoke<string | null>("choose_directory_path", {
        title: t("settings.privacy.trust.choose_folder_title"),
        initialPath: trustDirectoryPath.trim() || null,
      });
      if (selectedPath) {
        setTrustDirectoryPath(selectedPath);
        setBrowseTrustState("success");
      } else {
        setBrowseTrustState("idle");
      }
    } catch {
      setBrowseTrustState("error");
      setTrustError(t("settings.privacy.trust.browse_error"));
    } finally {
      setIsBrowsingTrustPath(false);
    }
  };

  const saveTrustFolder = async () => {
    const directoryPath = trustDirectoryPath.trim();
    setTrustMessage("");
    setTrustError("");

    if (!directoryPath) {
      setSaveTrustState("error");
      setTrustError(t("settings.privacy.trust.enter_folder"));
      return;
    }
    if (selectedCategories.length === 0) {
      setSaveTrustState("error");
      setTrustError(t("settings.privacy.trust.select_tool"));
      return;
    }

    setIsSavingTrust(true);
    setSaveTrustState("working");
    try {
      const authority = await invoke<{ proofId: string; persistence: string }>("request_native_authority", {
        request: {
          sessionId: authoritySessionId,
          operationClasses: selectedCategories,
          scopes: [directoryPath],
          maxSteps: 1,
          persistence: trustTier,
          locale: language,
        },
      });
      await invoke<{ policyId: number; message: string }>(
        "upsert_sovereign_trust_policy",
        {
          request: {
            authorityProofId: authority.proofId,
            sessionId: authoritySessionId,
            directoryPath,
            allowedToolCategories: selectedCategories,
            permissionLevel: authority.persistence,
            expiresAtMs: null,
            dailyTokenCostLimit: null,
            dailyCpuSecondsLimit: null,
          },
        },
      );
      setTrustMessage(t("settings.privacy.trust.folder_saved"));
      setSaveTrustState("success");
      await loadTrustDashboard(true, true);
    } catch (error) {
      setSaveTrustState("error");
      setTrustError(errorMessage(error, t("settings.privacy.trust.save_error")));
    } finally {
      setIsSavingTrust(false);
    }
  };

  const editPolicy = (policy: SovereignTrustPolicyRecord) => {
    setTrustDirectoryPath(policy.directoryPath);
    setTrustTier(
      policy.permissionLevel === "global_trust" ? "global_trust" : "session_gated",
    );
    setCategorySelection({
      shell_commands: policy.allowedToolCategories.includes("shell_commands"),
      external_writes: policy.allowedToolCategories.includes("external_writes"),
    });
    setTrustMessage("");
    setTrustError("");
    setSaveTrustState("idle");
  };

  const revokePolicy = async (policy: SovereignTrustPolicyRecord) => {
    setMutatingKey(`policy:${policy.id}`);
    setTrustMessage("");
    setTrustError("");
    try {
      const response = await invoke<SovereignTrustMutationResponse>(
        "revoke_sovereign_trust_policy",
        { policyId: policy.id },
      );
      setTrustMessage(
        response.affectedRows > 0 ? response.message : t("settings.privacy.trust.policy_already_revoked"),
      );
      await loadTrustDashboard(true, true);
    } catch (error) {
      setTrustError(errorMessage(error, t("settings.privacy.trust.policy_revoke_error")));
    } finally {
      setMutatingKey("");
    }
  };

  const revokeSession = async (session: SovereignTrustSessionRecord) => {
    setMutatingKey(`session:${session.id}`);
    setTrustMessage("");
    setTrustError("");
    try {
      const response = await invoke<SovereignTrustMutationResponse>(
        "revoke_sovereign_trust_session",
        { activeSessionId: session.id },
      );
      setTrustMessage(
        response.affectedRows > 0 ? response.message : t("settings.privacy.trust.session_already_revoked"),
      );
      await loadTrustDashboard(true, true);
    } catch (error) {
      setTrustError(errorMessage(error, t("settings.privacy.trust.session_revoke_error")));
    } finally {
      setMutatingKey("");
    }
  };

  const revokeReviewedScope = async (scope: ReviewedApprovalScope) => {
    setMutatingKey(`reviewed:${scope.grantId}`);
    try {
      await invoke("revoke_reviewed_approval_scope", { request: { grantId: scope.grantId, reason: "global_trust_surface_revocation" } });
      await loadTrustDashboard(true, true);
    } catch (error) {
      setTrustError(errorMessage(error, t("settings.privacy.trust.policy_revoke_error")));
    } finally { setMutatingKey(""); }
  };

  const revokeSessionScope = async (scope: SessionScopeTrustGrant) => {
    setMutatingKey(`session-scope:${scope.grantId}`);
    setTrustMessage("");
    setTrustError("");
    try {
      await invoke<boolean>("revoke_session_scope_trust_grant", {
        request: { grantId: scope.grantId },
      });
      setTrustMessage(t("settings.privacy.trust.session_access_removed"));
      await loadTrustDashboard(true, true);
    } catch (error) {
      setTrustError(errorMessage(error, t("settings.privacy.trust.session_revoke_error")));
    } finally {
      setMutatingKey("");
    }
  };

  const toggleCategory = (category: TrustToolCategory) => {
    setSaveTrustState("idle");
    setTrustMessage("");
    setTrustError("");
    setCategorySelection((current) => ({
      ...current,
      [category]: !current[category],
    }));
  };

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h2 className="text-sm font-semibold text-[var(--foreground)]">
            {t("settings.privacy.trust.title")}
          </h2>
          <div className="mt-2 flex flex-wrap gap-2 text-xs font-medium">
            <span className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.folders_count", { count: policies.length })}
            </span>
            <span className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.sessions_count", { count: activeSessions.length })}
            </span>
            {sessionScopeGrants.length > 0 ? <span className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[var(--foreground-muted)]">{t("permissions.scope_app_session")} · {sessionScopeGrants.length}</span> : null}
            <span className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.audit_rows_count", { count: auditEvents.length })}
            </span>
          </div>
        </div>
        <button
          className={secondaryButtonClass}
          disabled={isLoadingTrust}
          onClick={() => void loadTrustDashboard(false, true)}
          type="button"
        >
          {isLoadingTrust ? t("common.refreshing") : t("common.refresh")}
        </button>
      </div>

      <TrustFolderPolicyForm
        browseState={browseTrustState}
        categorySelection={categorySelection}
        directoryPath={trustDirectoryPath}
        isBrowsing={isBrowsingTrustPath}
        isSaving={isSavingTrust}
        onBrowse={() => void browseTrustFolder()}
        onCategoryToggle={toggleCategory}
        onDirectoryPathChange={(path) => {
          setTrustDirectoryPath(path);
          setBrowseTrustState("idle");
          setSaveTrustState("idle");
          setTrustMessage("");
          setTrustError("");
        }}
        onSave={() => void saveTrustFolder()}
        onTrustTierChange={(tier) => {
          setTrustTier(tier);
          setSaveTrustState("idle");
          setTrustMessage("");
          setTrustError("");
        }}
        saveState={saveTrustState}
        t={t}
        trustTier={trustTier}
      />

      {trustMessage && (
        <p aria-live="polite" className="mt-3 rounded-[var(--radius-sm)] border border-[var(--success)]/30 bg-[var(--success-background)] px-3 py-2 text-xs font-medium text-[var(--success)]" role="status">
          {trustMessage}
        </p>
      )}
      {trustError && (
        <p className="mt-3 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-xs font-medium text-[var(--destructive)]" role="alert">
          {trustError}
        </p>
      )}
      {trustLoadError && (
        <p className="mt-3 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-xs font-medium text-[var(--destructive)]" role="alert">
          {trustLoadError}
        </p>
      )}

      <div className="mt-5 grid gap-4 xl:grid-cols-[minmax(0,1.25fr)_minmax(18rem,0.75fr)]">
        <div className="min-w-0 rounded-[var(--radius-sm)] border border-[var(--border-soft)]">
          <div className="border-b border-[var(--border-soft)] px-4 py-3">
            <h3 className="text-xs font-semibold uppercase tracking-normal text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.trusted_folders")}
            </h3>
          </div>
          <div className="max-h-72 overflow-auto">
            {policies.length === 0 ? (
              <p className="px-4 py-5 text-sm text-[var(--foreground-muted)]">
                {t("settings.privacy.trust.no_trusted_folders")}
              </p>
            ) : (
              <div className="min-w-[860px]">
                {policies.map((policy) => (
                  <div
                    className="grid grid-cols-[minmax(16rem,1fr)_7rem_12rem_10rem_10rem_7rem_9rem] items-center gap-3 border-b border-[var(--border-soft)] px-4 py-3 last:border-b-0"
                    key={policy.id}
                  >
                    <div className="min-w-0">
                      <p
                        className="truncate font-mono text-xs text-[var(--foreground)]"
                        title={policy.canonicalDirectoryPath}
                      >
                        {policy.directoryPath}
                      </p>
                      <p className="mt-1 text-xs text-[var(--foreground-muted)]">
                        {t("settings.privacy.trust.last_used", {
                          time: formatTimestamp(policy.lastUsedAtMs, t),
                        })}
                      </p>
                    </div>
                    <span className="text-xs font-semibold text-[var(--foreground)]">
                      {formatTrustTier(policy.permissionLevel, t)}
                    </span>
                    <span className="text-xs text-[var(--foreground-muted)]">
                      {formatToolCategories(policy.allowedToolCategories, t)}
                    </span>
                    <span className="text-xs text-[var(--foreground-muted)]">
                      {formatUsage(
                        policy.estimatedTokenCostReservedToday,
                        policy.dailyTokenCostLimit,
                        t("settings.privacy.trust.estimated_tokens_reserved_unit"),
                      )}
                    </span>
                    <span className="text-xs text-[var(--foreground-muted)]">
                      {formatUsage(
                        policy.cpuSecondsReservedToday,
                        policy.dailyCpuSecondsLimit,
                        t("settings.privacy.trust.cpu_seconds_reserved_unit"),
                      )}
                    </span>
                    <span
                      className={`text-xs font-semibold ${
                        policy.active
                          ? "text-[var(--success)]"
                          : "text-[var(--foreground-muted)]"
                      }`}
                    >
                      {policy.active ? t("common.active") : t("settings.privacy.trust.expired")}
                    </span>
                    <div className="flex justify-end gap-2">
                      <button
                        className={secondaryButtonClass}
                        onClick={() => editPolicy(policy)}
                        type="button"
                      >
                        {t("common.edit")}
                      </button>
                      <button
                        className={secondaryButtonClass}
                        disabled={mutatingKey === `policy:${policy.id}`}
                        onClick={() => void revokePolicy(policy)}
                        type="button"
                      >
                        {t("settings.privacy.trust.revoke")}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="min-w-0 rounded-[var(--radius-sm)] border border-[var(--border-soft)]">
          <div className="border-b border-[var(--border-soft)] px-4 py-3">
            <h3 className="text-xs font-semibold uppercase tracking-normal text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.active_sessions")}
            </h3>
          </div>
          <div className="max-h-72 overflow-auto">
            {activeSessions.length === 0 && sessionScopeGrants.length === 0 ? (
              <p className="px-4 py-5 text-sm text-[var(--foreground-muted)]">
                {t("settings.privacy.trust.no_active_sessions")}
              </p>
            ) : (
              <>
              {sessionScopeGrants.map((scope) => (
                <div
                  className="border-b border-[var(--border-soft)] px-4 py-3 last:border-b-0"
                  key={scope.grantId}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-xs font-medium text-[var(--foreground)]" title={scope.canonicalResource}>
                        {scope.canonicalResource}
                      </p>
                      <p className="mt-1 text-xs text-[var(--foreground-muted)]">
                        {formatTrustAction(scope.actionClass, t)} · {t("permissions.scope_app_session")}
                      </p>
                    </div>
                    <button
                      className={secondaryButtonClass}
                      disabled={mutatingKey === `session-scope:${scope.grantId}`}
                      onClick={() => void revokeSessionScope(scope)}
                      type="button"
                    >
                      {t("settings.privacy.trust.revoke")}
                    </button>
                  </div>
                </div>
              ))}
              {activeSessions.map((session) => (
                <div
                  className="border-b border-[var(--border-soft)] px-4 py-3 last:border-b-0"
                  key={session.id}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p
                        className="truncate font-mono text-xs text-[var(--foreground)]"
                        title={session.canonicalDirectoryPath}
                      >
                        {session.directoryPath}
                      </p>
                      <p className="mt-1 text-xs text-[var(--foreground-muted)]">
                        {t("settings.privacy.trust.until", {
                          tier: formatTrustTier(session.permissionLevel, t),
                          time: formatTimestamp(session.expiresAtMs, t),
                        })}
                      </p>
                      <p className="mt-1 truncate text-xs text-[var(--foreground-muted)]">
                        {session.sessionId}
                      </p>
                    </div>
                    <button
                      className={secondaryButtonClass}
                      disabled={mutatingKey === `session:${session.id}`}
                      onClick={() => void revokeSession(session)}
                      type="button"
                    >
                      {t("settings.privacy.trust.revoke")}
                    </button>
                  </div>
                </div>
              ))}
              </>
            )}
          </div>
        </div>
      </div>

      <div className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)]">
        <div className="border-b border-[var(--border-soft)] px-4 py-3"><h3 className="text-xs font-semibold uppercase tracking-normal text-[var(--foreground-muted)]">{t("settings.privacy.trust.reviewed_scopes")}</h3></div>
        {reviewedScopes.length === 0 ? <p className="px-4 py-5 text-sm text-[var(--foreground-muted)]">{t("settings.privacy.trust.reviewed_scopes_empty")}</p> : <div className="max-h-72 overflow-auto">{reviewedScopes.map((scope) => <div className="flex items-center justify-between gap-4 border-b border-[var(--border-soft)] px-4 py-3 last:border-b-0" key={scope.grantId}><div className="min-w-0"><p className="text-xs font-semibold">{formatTrustScopeKind(scope.scopeKind, t)} · {formatTrustAction(scope.actionClass, t)}</p><p className="mt-1 truncate text-[10px] text-[var(--foreground-muted)]" title={scope.canonicalResource}>{scope.canonicalResource}</p><p className="mt-1 text-[10px] text-[var(--foreground-muted)]">{isUntilRevokedReviewedScope(scope) ? t("settings.privacy.trust.reviewed_scope_until_removed") : <>{scope.usedCount}/{scope.maxUses} {t("settings.privacy.trust.reviewed_scope_uses")} · {scope.active ? formatTimestamp(scope.expiresAtMs, t) : t("settings.privacy.trust.reviewed_scope_expired")}</>}</p></div><button className={secondaryButtonClass} disabled={!scope.active || mutatingKey === `reviewed:${scope.grantId}`} onClick={() => void revokeReviewedScope(scope)} type="button">{t("settings.privacy.trust.reviewed_scope_revoke")}</button></div>)}</div>}
      </div>

      <div className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)]">
        <div className="border-b border-[var(--border-soft)] px-4 py-3">
          <h3 className="text-xs font-semibold uppercase tracking-normal text-[var(--foreground-muted)]">
            {t("settings.privacy.trust.execution_audit")}
          </h3>
        </div>
        <div className="max-h-80 overflow-auto">
          {auditEvents.length === 0 ? (
            <p className="px-4 py-5 text-sm text-[var(--foreground-muted)]">
              {t("settings.privacy.trust.no_audit_rows")}
            </p>
          ) : (
            <div className="min-w-[920px]">
              <div className="grid grid-cols-[8rem_8rem_9rem_minmax(13rem,1fr)_13rem_8rem] gap-3 border-b border-[var(--border-soft)] px-4 py-2 text-xs font-semibold uppercase tracking-normal text-[var(--foreground-muted)]">
                <span>{t("settings.privacy.trust.columns.time")}</span>
                <span>{t("settings.privacy.trust.columns.mode")}</span>
                <span>{t("settings.privacy.trust.columns.operation")}</span>
                <span>{t("settings.privacy.trust.columns.target")}</span>
                <span>{t("settings.privacy.trust.columns.execution_hash")}</span>
                <span>{t("settings.privacy.trust.columns.status")}</span>
              </div>
              {auditEvents.map((event) => (
                <div
                  className="grid grid-cols-[8rem_8rem_9rem_minmax(13rem,1fr)_13rem_8rem] items-center gap-3 border-b border-[var(--border-soft)] px-4 py-3 text-xs last:border-b-0"
                  key={event.id}
                >
                  <span className="text-[var(--foreground-muted)]">
                    {formatTimestamp(event.createdAtMs, t)}
                  </span>
                  <span className="font-semibold text-[var(--foreground)]">
                    {formatAuthorizationMode(event.authorizationMode, event.trustTier, t)}
                  </span>
                  <span className="text-[var(--foreground)]">
                    {formatTrustAction(event.operation, t)}
                  </span>
                  <span
                    className="truncate text-[var(--foreground-muted)]"
                    title={event.summary}
                  >
                    {event.targetPath ?? event.inputKind ?? event.summary}
                  </span>
                  <span
                    className="font-mono text-[var(--foreground-muted)]"
                    title={event.executionHash}
                  >
                    {shortHash(event.executionHash)}
                  </span>
                  <span className="font-semibold text-[var(--foreground)]">
                    {formatTrustStatus(event.status, t)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
