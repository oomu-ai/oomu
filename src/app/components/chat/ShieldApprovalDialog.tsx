"use client";

import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useI18n } from "@/context/I18nContext";
import type {
  ShieldApprovalDecisionOptions,
  ShieldApprovalRequest,
} from "@/lib/approvalContracts";
export type {
  ShieldApprovalDecision,
  ShieldApprovalDecisionOptions,
  ShieldApprovalRequest,
} from "@/lib/approvalContracts";

type ShieldApprovalDialogProps = {
  request: ShieldApprovalRequest;
  isResolving: boolean;
  onApprove: (options?: ShieldApprovalDecisionOptions) => void;
  onDeny: () => void;
  resolutionErrorKey?: string | null;
};

function availableApprovalScopeKinds(request: ShieldApprovalRequest) {
  if (request.mandatoryReconfirm || !request.scopeTrustAvailable) {
    return ["once"];
  }
  return request.approvalScopeKinds?.length
    ? request.approvalScopeKinds
    : ["once"];
}

function scopeMenuIsOpen(
  state: { approvalToken: string; open: boolean },
  approvalToken: string,
) {
  return state.approvalToken === approvalToken && state.open;
}

export function ShieldApprovalStatusDialog({
  request,
  onDismiss,
}: {
  request: ShieldApprovalRequest;
  onDismiss: () => void;
}) {
  const { t } = useI18n();
  const calendarCreatePreview = request.actionType.replaceAll("-", "_").toLowerCase()
    === "create_system_calendar"
    ? safeNativeSystemActionPreview("create_system_calendar", request.preview?.trim() ?? "", t)
    : null;
  const calendarName = calendarCreatePreview?.[0]?.value;
  const actionLabel = calendarName
    ? t("chat.recovery.calendar_create", { calendar: calendarName })
    : request.actionLabel;
  const semanticSummary = calendarName
    ? t("permissions.action_detail")
    : request.semanticSummary;
  return (
    <ApprovalDialogFrame
      description={t("permissions.shield.native_body")}
      eyebrow={t("permissions.shield.pending")}
      footer={(
        <button
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)]"
          data-approval-initial-focus
          data-oomu-native-approval-status="dismiss"
          onClick={onDismiss}
          type="button"
        >
          {t("permissions.shield.hide")}
        </button>
      )}
      onDismiss={onDismiss}
      title={t("permissions.shield.native_title")}
    >
      <div className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
        <p className="text-sm font-semibold">{actionLabel}</p>
        {semanticSummary ? (
          <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
            {semanticSummary}
          </p>
        ) : null}
      </div>
    </ApprovalDialogFrame>
  );
}

type ShieldTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

function genericApprovalCopy(actionType: string, t: ShieldTranslate) {
  const publicSearch = actionType === "public_web_search";
  return {
    detail: t(publicSearch ? "chat.shield.public_search_detail" : "permissions.action_detail"),
    reason: t(publicSearch ? "chat.shield.public_search_reason" : "permissions.action_reason"),
    title: t(publicSearch ? "chat.shield.public_search_title" : "permissions.action_title"),
  };
}

function exactCanonicalTargetPath(value: string | null | undefined) {
  if (
    !value
    || value.length > 4_096
    || /[\0\r\n]/u.test(value)
    || !/^(?:\/|[a-z]:[\\/])/iu.test(value)
  ) return "";
  return value;
}

function FileWriteApprovalLocation({
  fileWrite,
  targetPath,
  t,
}: {
  fileWrite: boolean;
  targetPath: string | null | undefined;
  t: ShieldTranslate;
}) {
  if (!fileWrite) return null;
  const path = exactCanonicalTargetPath(targetPath);
  if (!path) return null;
  return (
    <div
      aria-label={t("permissions.location")}
      className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3"
      data-oomu-file-write-approval-location="true"
      role="group"
    >
      <p className="text-xs font-semibold text-[var(--foreground-muted)]">{t("permissions.location")}</p>
      <p className="mt-1 break-all text-sm leading-6 text-[var(--foreground)]" data-oomu-file-write-target-path="true" id="oomu-file-write-target-path">{path}</p>
    </div>
  );
}

// Shield approvals authorize an application-level action by an opaque approval
// token. They are deliberately global overlays and must not mutate chat-scoped
// transcript or status state when their native event arrives.
export function ShieldApprovalDialog({
  request,
  isResolving,
  onApprove,
  onDeny,
  resolutionErrorKey,
}: ShieldApprovalDialogProps) {
  const { t } = useI18n();
  const [trustScopeState, setTrustScopeState] = useState({
    approvalToken: request.approvalToken,
    kind: "once",
  });
  const [scopeMenuState, setScopeMenuState] = useState({
    approvalToken: request.approvalToken,
    open: false,
  });
  const scopeMenuId = useId();
  const scopeMenuRef = useRef<HTMLDivElement>(null);
  const scopeMenuTriggerRef = useRef<HTMLButtonElement>(null);
  const scopeMenuInitialFocusRef = useRef<"first" | "last">("first");
  const rawPreview = request.preview?.trim() ?? "";
  const connectorWrite = request.actionType === "connector_write";
  const connectorTransmission = request.actionType === "connector_transmission";
  const documentExport = request.actionType === "artifact_export";
  const spreadsheetExport = request.actionType === "workbook_export" || request.actionType === "workbook_template_export";
  const presentationExport = request.actionType === "presentation_export";
  const appControl = request.actionType === "app_control";
  const appControlContext = appControl ? safeAppControlPreview(rawPreview) : null;
  const appControlApp = appControlContext?.appName ?? t("chat.shield.app_control_unknown_app");
  const appControlAction = t(appControlContext?.actionKind === "apple_event" ? "app_control_actions.activate_approval" : `chat.shield.app_control_actions.${appControlContext?.actionKind ?? "unknown"}`);
  const knownExport = documentExport || spreadsheetExport || presentationExport;
  const normalizedActionType = request.actionType.replaceAll("-", "_").toLowerCase();
  const genericCopy = genericApprovalCopy(normalizedActionType, t);
  const nativeSystemAction = [
    "create_system_calendar",
    "create_system_calendar_event",
    "draft_system_email",
    "prepare_release_recovery_agenda",
    "create_release_recovery_calendar_event",
    "draft_release_recovery_email",
  ].includes(normalizedActionType);
  const nativeSystemActionPreview = nativeSystemAction
    ? safeNativeSystemActionPreview(normalizedActionType, rawPreview, t)
    : null;
  const channelConfiguration = normalizedActionType === "configure_channel";
  const channelConfigurationContext = channelConfiguration
    ? safeConfigureChannelPreview(rawPreview)
    : null;
  const evidenceArtifactWrite = [
    "prepare_background_agent_comparison",
    "prepare_milestone_constraint_recovery_plan",
  ].includes(normalizedActionType);
  const channelPlatform = channelConfigurationContext
    ? t(`channels.platform_${channelConfigurationContext.platform}`)
    : t("channels.channel_generic");
  const channelConfigurationTitle = channelConfigurationContext?.ownerId
    ? t("trust.configure_channel_prompt", { platform: channelPlatform, owner: channelConfigurationContext.ownerId })
    : t("trust.configure_channel_prompt_no_owner", { platform: channelPlatform });
  const fileAccess = !channelConfiguration && !nativeSystemAction && (evidenceArtifactWrite || ["filesystem_read", "filesystem_write"].includes(request.actionClass ?? "") || ["file_read", "file_list", "file_write", "filesystem_read", "filesystem_write", "codebase_patch", "document_index"].includes(normalizedActionType));
  const fileWrite = evidenceArtifactWrite || request.actionClass === "filesystem_write" || ["file_write", "filesystem_write", "codebase_patch", "document_index"].includes(normalizedActionType);
  const genericAction = !fileAccess && !connectorWrite && !connectorTransmission && !knownExport && !appControl && !channelConfiguration && !nativeSystemAction;
  const fileResource = request.targetPath ?? request.canonicalResource ?? "";
  const resourceName = fileResource
    ? safeFileResourceName(fileResource, t)
    : t("permissions.selected_location");
  const fullScopeFolder = safeResourceValue(
    request.scopeTrustPrefix ?? request.canonicalResource ?? request.targetPath ?? "",
    true,
  );
  const scopeFolder = fullScopeFolder
    ? permissionResourceName(fullScopeFolder)
    : "";
  const genericContext = genericAction
    ? safeGenericActionContext(request, normalizedActionType)
    : "";
  const transmissionDestination = safeApprovalDetailResource(
    request.canonicalResource ?? request.principal ?? "",
    t,
  );
  const detailTargetPath = request.targetPath
    ? safeApprovalDetailResource(request.targetPath, t) : "";
  const detailScopePrefix = request.scopeTrustPrefix
    ? safeApprovalDetailResource(request.scopeTrustPrefix, t)
    : "";
  const detailCanonicalResource = request.canonicalResource
    ? safeApprovalDetailResource(request.canonicalResource, t)
    : "";
  const actionLabel = connectorWrite
    ? connectorActionLabel(request.actionLabel, t)
    : connectorTransmission
      ? t("chat.shield.connector_transmission_action")
    : documentExport
      ? t("chat.shield.document_export_action")
      : spreadsheetExport
        ? t("chat.shield.spreadsheet_export_action")
        : presentationExport
          ? t("chat.shield.presentation_export_action")
          : appControl
            ? t("chat.shield.app_control_action", { action: appControlAction, app: appControlApp })
          : channelConfiguration && channelConfigurationContext
            ? t(channelConfigurationContext.isActive ? "tools.configure_channel.activate" : "tools.configure_channel.deactivate", { platform: channelPlatform })
          : nativeSystemAction
            ? normalizedActionType === "create_system_calendar"
              ? t("chat.recovery.calendar_create", {
                  calendar: nativeSystemActionPreview?.[0]?.value ?? t("chat.recovery.calendar_unknown"),
                })
              : normalizedActionType === "prepare_release_recovery_agenda"
                ? t("chat.recovery.approval_create_agenda")
                : t(["draft_system_email", "draft_release_recovery_email"].includes(normalizedActionType)
                  ? "settings.privacy.trust.action_save_mail_draft"
                  : "settings.privacy.trust.action_add_calendar_event")
        : genericActionLabel(request, normalizedActionType, t);
  const approvalTitle = fileAccess
    ? t("permissions.file_title", { name: resourceName })
    : connectorWrite
    ? t("chat.shield.connector_write_title", { action: actionLabel })
    : connectorTransmission
      ? t("chat.shield.connector_transmission_title")
    : documentExport
      ? t("chat.shield.document_export_title")
      : spreadsheetExport
        ? t("chat.shield.spreadsheet_export_title")
        : presentationExport
          ? t("chat.shield.presentation_export_title")
          : appControl
            ? t("chat.shield.app_control_title", { app: appControlApp })
          : channelConfiguration && channelConfigurationContext
            ? channelConfigurationTitle
        : genericCopy.title;
  const approvalDetail = fileAccess
    ? t(fileWrite ? "permissions.file_write_detail" : "permissions.file_read_detail")
    : connectorWrite
    ? t("chat.shield.connector_write_detail", { action: actionLabel })
    : connectorTransmission
      ? t("chat.shield.connector_transmission_detail", { destination: transmissionDestination })
    : documentExport
      ? t("chat.shield.document_export_detail")
      : spreadsheetExport
        ? t("chat.shield.spreadsheet_export_detail")
        : presentationExport
          ? t("chat.shield.presentation_export_detail")
          : appControl
            ? t("chat.shield.app_control_detail", { action: appControlAction, app: appControlApp })
          : channelConfiguration
            ? t("chat.shield.configure_channel_detail")
        : genericCopy.detail;
  const approvalReason = fileAccess
    ? t("permissions.file_reason")
    : connectorWrite
    ? t("chat.shield.connector_write_reason")
    : connectorTransmission
      ? t("chat.shield.connector_transmission_reason")
    : knownExport
      ? t("chat.shield.export_reason")
      : appControl
        ? t("chat.shield.app_control_reason")
      : channelConfiguration
        ? t("chat.shield.configure_channel_reason")
      : genericCopy.reason;
  const connectorPreviewRows = connectorWrite
    ? safeConnectorPreview(rawPreview, request.actionLabel, actionLabel, t)
    : { rows: [], verified: false };
  const transmissionPreview = connectorTransmission ? safeTransmissionPreview(rawPreview, request.canonicalResource ?? request.principal ?? "", t) : null;
  const connectorWriteReady = !connectorWrite || (
    CONNECTOR_ACTIONS.has(request.actionLabel) && connectorPreviewRows.verified
  );
  const genericActionReady = !genericAction || isKnownGenericAction(
    request,
    normalizedActionType,
  );
  const transmissionReady = !connectorTransmission || transmissionPreview !== null;
  const appControlReady = !appControl || Boolean(appControlContext);
  const channelConfigurationReady = !channelConfiguration || Boolean(channelConfigurationContext);
  const nativeSystemActionReady = !nativeSystemAction || Boolean(nativeSystemActionPreview);
  const trustScopeKind =
    trustScopeState.approvalToken === request.approvalToken
      ? trustScopeState.kind
      : "once";
  const scopeKinds = availableApprovalScopeKinds(request);
  const scopeMenuOpen = scopeMenuIsOpen(scopeMenuState, request.approvalToken);
  const primaryLabel = fileAccess
    ? t("permissions.scope_once")
    : t("chat.shield.approve");

  useEffect(() => {
    if (!scopeMenuOpen) return;
    const items = Array.from(
      scopeMenuRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not([disabled])',
      ) ?? [],
    );
    const target = scopeMenuInitialFocusRef.current === "last"
      ? items.at(-1)
      : items[0];
    target?.focus();
  }, [request.approvalToken, scopeMenuOpen]);

  function openScopeMenu(initialFocus: "first" | "last" = "first") {
    if (isResolving) return;
    scopeMenuInitialFocusRef.current = initialFocus;
    setScopeMenuState({ approvalToken: request.approvalToken, open: true });
  }

  function closeScopeMenu(restoreFocus: boolean) {
    const dialog = scopeMenuRef.current?.closest<HTMLElement>('[role="dialog"]');
    setScopeMenuState({ approvalToken: request.approvalToken, open: false });
    if (restoreFocus) {
      window.setTimeout(() => {
        const trigger = scopeMenuTriggerRef.current;
        if (trigger && !trigger.disabled) trigger.focus();
        else dialog?.focus();
      }, 0);
    }
  }

  function handleScopeMenuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeScopeMenu(true);
      return;
    }
    if (event.key === "Tab") {
      window.setTimeout(() => closeScopeMenu(false), 0);
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const items = Array.from(
      scopeMenuRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not([disabled])',
      ) ?? [],
    );
    if (!items.length) return;
    event.preventDefault();
    const currentIndex = items.findIndex((item) => item === document.activeElement);
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? items.length - 1
        : event.key === "ArrowUp"
          ? (currentIndex <= 0 ? items.length - 1 : currentIndex - 1)
          : (currentIndex + 1) % items.length;
    items[nextIndex]?.focus();
  }

  return (
    <ApprovalDialogFrame
      description={<><p>{approvalDetail}</p><p className="mt-2">{approvalReason}</p></>}
      eyebrow={t("chat.shield.paused")}
      footer={<>
        <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50" data-approval-initial-focus disabled={isResolving} onClick={onDeny} type="button">
          {fileAccess ? t("permissions.dont_allow") : t("chat.shield.deny")}
        </button>
        {fileAccess && scopeKinds.length > 1 ? (
          <div className="relative inline-flex" onKeyDown={(event) => {
            if (event.key === "Escape" && scopeMenuOpen) {
              event.preventDefault();
              event.stopPropagation();
              closeScopeMenu(true);
            }
          }}>
            <button aria-busy={isResolving} className="rounded-l-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50" data-action-state={isResolving ? "working" : "idle"} disabled={isResolving} id="oomu-file-write-approve-once" onClick={() => { closeScopeMenu(false); onApprove({ trustScope: false, trustScopeKind: "once" }); }} type="button">
              {isResolving ? t("chat.shield.resolving") : primaryLabel}
            </button>
            <button aria-controls={scopeMenuId} aria-expanded={scopeMenuOpen} aria-haspopup="menu" aria-label={t("permissions.more_access_options")} className="rounded-r-[var(--radius-sm)] border-l border-white/25 bg-[var(--accent)] px-2.5 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50" disabled={isResolving} onClick={() => { if (scopeMenuOpen) closeScopeMenu(false); else openScopeMenu(); }} onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); openScopeMenu(event.key === "ArrowUp" ? "last" : "first"); } }} ref={scopeMenuTriggerRef} type="button">
              <svg aria-hidden className="h-4 w-4" fill="none" viewBox="0 0 16 16">
                <path d="m4 6 4 4 4-4" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.5" />
              </svg>
            </button>
            {scopeMenuOpen ? (
              <div className="absolute bottom-[calc(100%+0.5rem)] right-0 z-10 min-w-56 overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-1 shadow-xl" id={scopeMenuId} onKeyDown={handleScopeMenuKeyDown} ref={scopeMenuRef} role="menu">
                {scopeKinds.filter((kind) => kind !== "once").map((kind) => (
                  <button className="block w-full rounded-[var(--radius-sm)] px-3 py-2 text-left text-sm font-medium hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50" disabled={isResolving} key={kind} onClick={() => { setScopeMenuState({ approvalToken: request.approvalToken, open: false }); scopeMenuTriggerRef.current?.focus(); onApprove({ trustScope: true, trustScopeKind: kind }); }} role="menuitem" type="button">
                    <span className="block">{approvalScopeLabel(kind, t, true)}</span>
                    {scopeFolder ? <span className="mt-0.5 block break-all text-xs font-normal text-[var(--foreground-muted)]">{scopeFolder}</span> : null}
                  </button>
                ))}
              </div>
            ) : null}
          </div>
        ) : (
          <button aria-busy={isResolving} className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50" data-action-state={isResolving ? "working" : "idle"} disabled={isResolving || !connectorWriteReady || !genericActionReady || !transmissionReady || !appControlReady || !channelConfigurationReady || !nativeSystemActionReady} id={[undefined, "oomu-file-write-approve-once"][Number(fileWrite)]} onClick={() => onApprove({ trustScope: trustScopeKind !== "once", trustScopeKind })} type="button">
            {isResolving ? t("chat.shield.resolving") : primaryLabel}
          </button>
        )}
      </>}
      id="oomu-shield-approval-dialog" onDismiss={() => { if (!isResolving) onDeny(); }}
      title={approvalTitle}
    >
      {connectorPreviewRows.rows.length ? <details className="mt-4 rounded border border-[var(--border-soft)] p-3"><summary className="cursor-pointer text-sm font-semibold">{t("chat.shield.connector_write_details")}</summary><dl className="mt-3 grid gap-2">{connectorPreviewRows.rows.map((row, index) => <div className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-2 text-xs" key={`${row.label}-${index}`}><dt className="font-semibold text-[var(--foreground-muted)]">{row.label}</dt><dd className="mt-1 break-words text-[var(--foreground)]">{row.value}</dd></div>)}</dl></details> : null}
      {transmissionPreview ? <details className="mt-4 rounded border border-[var(--border-soft)] p-3"><summary className="cursor-pointer text-sm font-semibold">{t("chat.shield.connector_transmission_details")}</summary><div className="mt-3 grid gap-3 text-xs"><div className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-2"><p className="font-semibold text-[var(--foreground-muted)]">{t("chat.shield.connector_fields.destination")}</p><p className="mt-1 break-words text-[var(--foreground)]">{transmissionPreview.destination}</p></div><div><p className="font-semibold text-[var(--foreground-muted)]">{t("chat.shield.connector_data_included")}</p><ul className="mt-2 grid gap-1">{transmissionPreview.dataLabels.map((label) => <li className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-2 text-[var(--foreground)]" key={label}>{label}</li>)}</ul></div></div></details> : null}
      {connectorTransmission && !transmissionReady ? <p className="mt-4 rounded bg-[var(--warning-background)] p-3 text-xs font-semibold text-[var(--warning)]">{t("chat.shield.connector_transmission_unavailable")}</p> : null}
      {appControl && !appControlReady ? <p className="mt-4 rounded bg-[var(--warning-background)] p-3 text-xs font-semibold text-[var(--warning)]">{t("chat.shield.app_control_unavailable")}</p> : null}
      {(!connectorWriteReady || !genericActionReady || !channelConfigurationReady || !nativeSystemActionReady) ? <p className="mt-4 rounded bg-[var(--warning-background)] p-3 text-xs font-semibold text-[var(--warning)]" role="alert">{t("chat.shield.action_unavailable")}</p> : null}
      <FileWriteApprovalLocation fileWrite={fileWrite} targetPath={request.targetPath} t={t} />

      {!fileAccess ? <div className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-3 text-sm">
        <p className="font-semibold">{actionLabel}</p>
        {genericContext ? <p className="mt-1 break-words text-xs leading-5 text-[var(--foreground-muted)]">{genericContext}</p> : null}
        {nativeSystemActionPreview ? <dl className="mt-3 grid max-h-64 gap-2 overflow-y-auto">{nativeSystemActionPreview.map((row, index) => <div className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-2 text-xs" key={`${row.labelKey}-${index}`}><dt className="font-semibold text-[var(--foreground-muted)]">{t(row.labelKey)}</dt><dd className="mt-1 whitespace-pre-wrap break-words text-[var(--foreground)]" data-oomu-approval-detail={row.labelKey}>{row.value || t("mcp_confirmation.none")}</dd></div>)}</dl> : null}
      </div> : null}

      {scopeKinds.length > 1 && !fileAccess ? (
        <label className="mt-5 grid gap-2">
          <span className="text-sm font-semibold">{fileAccess ? t("permissions.duration") : t("chat.shield.scope_title")}</span>
          <select className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm" disabled={isResolving} onChange={(event) => setTrustScopeState({ approvalToken: request.approvalToken, kind: event.target.value })} value={trustScopeKind}>
            {scopeKinds.map((kind) => <option key={kind} value={kind}>{approvalScopeLabel(kind, t)}</option>)}
          </select>
        </label>
      ) : null}
      {request.mandatoryReconfirm ? <p className="mt-5 rounded-[var(--radius-sm)] bg-[var(--warning-background)] p-3 text-xs font-semibold">{t("chat.shield.always_confirm")}</p> : null}
      {resolutionErrorKey ? <p className="mt-4 rounded-[var(--radius-sm)] bg-[var(--destructive-background)] p-3 text-sm text-[var(--destructive)]" role="alert">{t(resolutionErrorKey)}</p> : null}

      <details className="group mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)]">
        <summary className="cursor-pointer list-none px-3 py-2.5 text-sm font-semibold [&::-webkit-details-marker]:hidden">{t("common.details")}</summary>
        <div className="space-y-4 border-t border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
          <dl className="grid gap-3 text-xs sm:grid-cols-2">
            <div><dt className="text-[var(--foreground-subtle)]">{t("chat.shield.category")}</dt><dd className="mt-1 font-medium">{shieldRiskLabel(request.riskTier, t)}</dd></div>
            <div><dt className="text-[var(--foreground-subtle)]">{t("chat.shield.approval")}</dt><dd className="mt-1 font-medium">{shieldApprovalTierLabel(request.approvalTier ?? request.approvalMode ?? request.riskTier, t)}</dd></div>
            <div><dt className="text-[var(--foreground-subtle)]">{t("chat.shield.requested")}</dt><dd className="mt-1 font-medium">{formatApprovalTime(request.requestedAtMs, t)}</dd></div>
            {detailTargetPath ? <div className="sm:col-span-2"><dt className="text-[var(--foreground-subtle)]">{t("permissions.location")}</dt><dd className="mt-1 break-all leading-5" data-oomu-approval-detail="target-path">{detailTargetPath}</dd></div> : null}
            {request.scopeTrustPrefix && request.scopeTrustPrefix !== request.targetPath && detailScopePrefix ? <div className="sm:col-span-2"><dt className="text-[var(--foreground-subtle)]">{t("chat.shield.scope_title")}</dt><dd className="mt-1 break-all leading-5" data-oomu-approval-detail="scope-prefix">{detailScopePrefix}</dd></div> : null}
            {request.canonicalResource && request.canonicalResource !== request.targetPath && request.canonicalResource !== request.scopeTrustPrefix && detailCanonicalResource ? <div className="sm:col-span-2"><dt className="text-[var(--foreground-subtle)]">{t("chat.shield.connector_fields.destination")}</dt><dd className="mt-1 break-all leading-5" data-oomu-approval-detail="canonical-resource">{detailCanonicalResource}</dd></div> : null}
          </dl>
        </div>
      </details>
    </ApprovalDialogFrame>
  );
}

function safeConfigureChannelPreview(value: string) {
  try {
    const preview = JSON.parse(value) as Record<string, unknown>;
    const platform = typeof preview.platform === "string"
      ? preview.platform.trim().toLowerCase()
      : "";
    const ownerId = typeof preview.ownerId === "string"
      ? safeDisplayText(preview.ownerId, 256)
      : "";
    if (!["telegram", "discord", "slack"].includes(platform) || typeof preview.isActive !== "boolean") {
      return null;
    }
    return { platform, ownerId, isActive: preview.isActive };
  } catch {
    return null;
  }
}

function approvalScopeLabel(
  kind: string,
  t: (key: string) => string,
  fileAccess = false,
) {
  if (kind === "once") {
    return t(fileAccess ? "permissions.scope_once" : "chat.shield.scope_once");
  }
  if (kind === "chat_session") return t("chat.shield.scope_chat_session");
  if (kind === "app_session") return t("permissions.scope_app_session");
  if (kind === "persistent") {
    return t(fileAccess ? "permissions.scope_persistent" : "chat.shield.scope_persistent");
  }
  const key = ["task", "project_path"].includes(kind) ? kind : "unknown";
  return t(`chat.shield.scope_${key}`);
}

function permissionResourceName(value: string) {
  const normalized = value.replace(/[\\/]+$/, "");
  return normalized.split(/[\\/]/).filter(Boolean).at(-1) ?? value;
}

function safeFileResourceName(value: string, t: (key: string) => string) {
  const resource = safeResourceValue(value, true);
  if (!resource) return t("common.unknown");
  return safeDisplayText(permissionResourceName(resource), 96) || t("common.unknown");
}

function safeApprovalDetailResource(
  value: string,
  t: (key: string) => string,
) {
  return safeResourceValue(value, true) || t("common.unknown");
}

function safeResourceValue(value: string, allowFullPath: boolean) {
  const normalized = safeDisplayText(value, 512);
  if (!normalized) return "";
  try {
    const url = new URL(normalized);
    if (url.protocol === "http:" || url.protocol === "https:") {
      return safeDisplayText(url.hostname.replace(/^www\./i, ""), 120);
    }
    if (url.protocol === "file:") {
      const filePath = decodeURIComponent(url.pathname);
      return safeAbsolutePath(filePath, allowFullPath);
    }
    return "";
  } catch {
    // Paths, hostnames, and human labels are handled below.
  }
  if (looksOpaqueOrCodeLikeResource(normalized)) return "";
  const absolutePath = safeAbsolutePath(normalized, allowFullPath);
  if (absolutePath) return absolutePath;
  if (/^(?:[a-z0-9](?:[a-z0-9-]{0,62})\.)+[a-z]{2,63}$/i.test(normalized)) {
    return normalized.replace(/^www\./i, "");
  }
  return /^[\p{L}\p{N}][\p{L}\p{N} .'’()-]{0,95}$/u.test(normalized)
    ? normalized
    : "";
}

function safeAbsolutePath(value: string, allowFullPath: boolean) {
  if (!/^(?:\/|~\/|[a-z]:[\\/])/i.test(value)) return "";
  if (looksOpaqueOrCodeLikeResource(value)) return "";
  return allowFullPath ? value : permissionResourceName(value);
}

function looksOpaqueOrCodeLikeResource(value: string) {
  return (
    /[`{}[\];=]/.test(value) ||
    /(?:^|[\\/._-])[a-f0-9]{24,}(?:$|[\\/._-])/i.test(value) ||
    /(?:^|[^a-z0-9])[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}(?:$|[^a-z0-9])/i.test(value) ||
    /(?:^|[.\/])eyJ[a-z0-9_-]{16,}(?:\.|$)/i.test(value)
  );
}

type NativeSystemActionPreviewRow = { labelKey: string; value: string };

function safeNativeSystemActionPreview(
  actionType: string,
  rawPreview: string,
  t: (key: string, variables?: Record<string, string | number>) => string,
): NativeSystemActionPreviewRow[] | null {
  let value: unknown;
  try {
    value = JSON.parse(rawPreview);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const object = value as Record<string, unknown>;
  const exactKeys = (keys: string[]) =>
    Object.keys(object).every((key) => keys.includes(key));
  const text = (key: string, maximum: number, required = false) => {
    const candidate = object[key];
    if (candidate === undefined && !required) return "";
    if (
      typeof candidate !== "string" ||
      candidate.includes("\0") ||
      candidate.length > maximum ||
      (required && !candidate.trim())
    ) return null;
    return candidate;
  };
  const integer = (key: string) => {
    const candidate = object[key];
    return typeof candidate === "number" && Number.isSafeInteger(candidate)
      ? candidate
      : null;
  };
  const exactDate = (key: string) => {
    const candidate = text(key, 64, true);
    if (
      !candidate ||
      !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(?:Z|[+-]\d{2}:\d{2})$/u.test(candidate) ||
      !Number.isFinite(Date.parse(candidate))
    ) return null;
    return candidate;
  };
  const sha256 = (key: string) => {
    const candidate = text(key, 64, true);
    return candidate && /^[a-f0-9]{64}$/u.test(candidate) ? candidate : null;
  };
  if (actionType === "prepare_release_recovery_agenda") {
    if (!exactKeys([
      "inputPath",
      "outputPath",
      "day",
      "windowStartLocal",
      "windowEndLocal",
      "durationMinutes",
      "agendaItemCount",
      "locale",
    ])) return null;
    const inputPath = text("inputPath", 4_096, true);
    const outputPath = text("outputPath", 4_096, true);
    const day = text("day", 32, true);
    const windowStart = text("windowStartLocal", 5, true);
    const windowEnd = text("windowEndLocal", 5, true);
    const duration = integer("durationMinutes");
    const itemCount = integer("agendaItemCount");
    const locale = text("locale", 16, true);
    if (
      !inputPath?.startsWith("/") ||
      !outputPath?.startsWith("/") ||
      inputPath === outputPath ||
      day !== "next_weekday" ||
      windowStart !== "13:00" ||
      windowEnd !== "16:00" ||
      duration !== 30 ||
      itemCount !== 5 ||
      !locale ||
      !/^[a-z]{2}(?:-[A-Z]{2})?$/u.test(locale)
    ) return null;
    return [
      { labelKey: "chat.recovery.approval_input_file", value: inputPath },
      { labelKey: "chat.recovery.approval_output_file", value: outputPath },
      { labelKey: "chat.recovery.approval_day", value: t("chat.recovery.approval_next_weekday") },
      { labelKey: "chat.recovery.approval_time_window", value: `${windowStart}–${windowEnd}` },
      { labelKey: "chat.recovery.approval_duration", value: t("chat.recovery.approval_minutes", { count: duration }) },
      { labelKey: "chat.recovery.approval_agenda_items", value: t("chat.recovery.approval_items", { count: itemCount }) },
      { labelKey: "chat.recovery.approval_existing_output", value: t("chat.recovery.approval_preserve_existing") },
    ];
  }
  if (actionType === "create_release_recovery_calendar_event") {
    if (!exactKeys([
      "calendarName",
      "title",
      "startDate",
      "endDate",
      "location",
      "notes",
      "availability",
      "agendaStep",
      "agendaSha256",
      "outputPath",
      "outputSha256",
      "byteLength",
    ])) return null;
    const calendar = text("calendarName", 512, true);
    const title = text("title", 2_048, true);
    const start = exactDate("startDate");
    const end = exactDate("endDate");
    const location = text("location", 2_048);
    const notes = text("notes", 16_384, true);
    const availability = text("availability", 16, true);
    const agendaStep = integer("agendaStep");
    const agendaSha = sha256("agendaSha256");
    const outputPath = text("outputPath", 4_096, true);
    const outputSha = sha256("outputSha256");
    const byteLength = integer("byteLength");
    if (
      !calendar || !title || !start || !end || location === null || !notes ||
      availability !== "tentative" || agendaStep !== 0 || !agendaSha ||
      !outputPath?.startsWith("/") || outputSha !== agendaSha ||
      byteLength === null || byteLength < 1 || byteLength > 1_048_576 ||
      Date.parse(end) <= Date.parse(start)
    ) return null;
    return [
      { labelKey: "chat.shield.native_fields.calendar", value: calendar },
      { labelKey: "workflow.storyboard.fields.title", value: title },
      { labelKey: "chat.shield.connector_fields.start", value: start },
      { labelKey: "chat.shield.connector_fields.end", value: end },
      { labelKey: "mcp_confirmation.location", value: location! },
      { labelKey: "chat.shield.native_fields.notes", value: notes },
      { labelKey: "chat.shield.native_fields.availability", value: availability },
      { labelKey: "chat.recovery.approval_output_file", value: outputPath },
      { labelKey: "chat.recovery.approval_event_count", value: t("chat.recovery.approval_one_tentative_event") },
    ];
  }
  if (actionType === "draft_release_recovery_email") {
    if (!exactKeys([
      "to",
      "subject",
      "body",
      "startDate",
      "endDate",
      "agendaItems",
      "agendaStep",
      "calendarStep",
      "agendaSha256",
      "outputPath",
      "outputSha256",
      "byteLength",
    ])) return null;
    const to = text("to", 4_096, true);
    const subject = text("subject", 998, true);
    const body = text("body", 20_000, true);
    const start = exactDate("startDate");
    const end = exactDate("endDate");
    const agendaItems = object.agendaItems;
    const agendaStep = integer("agendaStep");
    const calendarStep = integer("calendarStep");
    const agendaSha = sha256("agendaSha256");
    const outputPath = text("outputPath", 4_096, true);
    const outputSha = sha256("outputSha256");
    const byteLength = integer("byteLength");
    if (
      !to || !subject || !body || !start || !end ||
      Date.parse(end) <= Date.parse(start) ||
      !Array.isArray(agendaItems) || agendaItems.length !== 5 ||
      !agendaItems.every((item) => typeof item === "string" && item.trim() && item.length <= 2_048) ||
      agendaStep !== 0 || calendarStep !== 1 || !agendaSha ||
      !outputPath?.startsWith("/") || outputSha !== agendaSha ||
      byteLength === null || byteLength < 1 || byteLength > 1_048_576
    ) return null;
    return [
      { labelKey: "chat.shield.connector_fields.to", value: to },
      { labelKey: "chat.shield.connector_fields.subject", value: subject },
      { labelKey: "chat.shield.connector_fields.body", value: body },
      { labelKey: "chat.shield.connector_fields.start", value: start },
      { labelKey: "chat.shield.connector_fields.end", value: end },
      { labelKey: "chat.recovery.approval_agenda_items", value: (agendaItems as string[]).join("\n") },
      { labelKey: "chat.recovery.approval_output_file", value: outputPath },
      { labelKey: "chat.recovery.approval_delivery", value: t("chat.recovery.approval_save_draft_only") },
      { labelKey: "chat.recovery.approval_delivery", value: t("chat.recovery.approval_will_not_send") },
    ];
  }
  if (actionType === "draft_system_email") {
    if (!exactKeys(["to", "cc", "bcc", "subject", "body"])) return null;
    const to = text("to", 4_096);
    const cc = text("cc", 4_096);
    const bcc = text("bcc", 4_096);
    const subject = text("subject", 998, true);
    const body = text("body", 20_000, true);
    if ([to, cc, bcc, subject, body].some((entry) => entry === null)) return null;
    return [
      { labelKey: "chat.shield.connector_fields.to", value: to! },
      { labelKey: "chat.shield.connector_fields.cc", value: cc! },
      { labelKey: "chat.shield.native_fields.bcc", value: bcc! },
      { labelKey: "chat.shield.connector_fields.subject", value: subject! },
      { labelKey: "chat.shield.connector_fields.body", value: body! },
    ];
  }
  if (actionType === "create_system_calendar") {
    if (!exactKeys(["calendarName"])) return null;
    const calendar = text("calendarName", 80, true);
    if (calendar === null) return null;
    return [{ labelKey: "chat.shield.native_fields.calendar", value: calendar }];
  }
  if (actionType !== "create_system_calendar_event") return null;
  if (!exactKeys(["calendarName", "title", "startDate", "endDate", "location", "notes", "availability"])) return null;
  const calendar = text("calendarName", 512, true);
  const title = text("title", 2_048, true);
  const start = text("startDate", 64, true);
  const end = text("endDate", 64, true);
  const location = text("location", 2_048);
  const notes = text("notes", 16_384);
  const availability = text("availability", 16, true);
  if (
    [calendar, title, start, end, location, notes, availability].some(
      (entry) => entry === null,
    ) ||
    !["busy", "free", "tentative"].includes(availability!)
  ) return null;
  return [
    { labelKey: "chat.shield.native_fields.calendar", value: calendar! },
    { labelKey: "workflow.storyboard.fields.title", value: title! },
    { labelKey: "chat.shield.connector_fields.start", value: start! },
    { labelKey: "chat.shield.connector_fields.end", value: end! },
    { labelKey: "mcp_confirmation.location", value: location! },
    { labelKey: "chat.shield.native_fields.notes", value: notes! },
    { labelKey: "chat.shield.native_fields.availability", value: availability! },
  ];
}

const GENERIC_ACTION_LABEL_KEYS: Record<string, string> = {
  approval_grant: "settings.privacy.trust.action_save_approval",
  delete_file: "settings.privacy.trust.action_delete_files",
  trash: "settings.privacy.trust.action_delete_files",
  trash_file: "settings.privacy.trust.action_delete_files",
  shell_command: "settings.privacy.trust.action_run_command",
  execute_command: "settings.privacy.trust.action_run_command",
  codebase_compile: "settings.privacy.trust.action_run_command",
  web_fetch: "settings.privacy.trust.action_use_network",
  network_request: "settings.privacy.trust.action_use_network",
  network_diagnostic: "settings.privacy.trust.action_check_network",
  mcp_tool_call: "settings.privacy.trust.action_connected_tool",
  mcp_connect_server: "settings.privacy.trust.action_connected_tool",
  mcp_execute_remote_tool: "settings.privacy.trust.action_connected_tool",
  system_audit: "settings.privacy.trust.action_check_system",
  airlock_export: "settings.privacy.trust.action_export",
  telemetry_archive: "settings.privacy.trust.action_export",
  prepare_background_agent_comparison: "trust.tool_kind.file_write",
  prepare_milestone_constraint_recovery_plan: "trust.tool_kind.file_write",
  routine_preauthorization: "settings.privacy.trust.action_save_approval",
  public_web_search: "chat.shield.public_search_action",
};

const BROWSER_ACTIONS = new Set([
  "browser_click",
  "browser_upload_approved_file",
  "browser_download_to_quarantine",
  "browser_download_export",
]);

function genericActionLabel(
  request: ShieldApprovalRequest,
  actionType: string,
  t: (key: string) => string,
) {
  const actionClass = request.actionClass?.trim().replaceAll("-", "_").toLowerCase() ?? "";
  if (BROWSER_ACTIONS.has(actionType) || BROWSER_ACTIONS.has(actionClass)) {
    return t("settings.privacy.trust.action_browser");
  }
  return t(
    GENERIC_ACTION_LABEL_KEYS[actionType] ??
      GENERIC_ACTION_LABEL_KEYS[actionClass] ??
      "settings.privacy.trust.action_other",
  );
}

function isKnownGenericAction(
  request: ShieldApprovalRequest,
  actionType: string,
) {
  const actionClass = request.actionClass
    ?.trim()
    .replaceAll("-", "_")
    .toLowerCase() ?? "";
  return BROWSER_ACTIONS.has(actionType) ||
    BROWSER_ACTIONS.has(actionClass) ||
    actionType in GENERIC_ACTION_LABEL_KEYS ||
    actionClass in GENERIC_ACTION_LABEL_KEYS;
}

function safeGenericActionContext(
  request: ShieldApprovalRequest,
  actionType: string,
) {
  const target = friendlyResourceContext(
    request.targetPath ?? request.canonicalResource ?? "",
  );
  const tool = actionType === "mcp_tool_call"
    ? readableToolContext(request.actionLabel)
    : ["mcp_execute_remote_tool", "mcp_connect_server"].includes(actionType)
      ? readableToolContext(request.principal ?? request.actionLabel)
      : "";
  return [tool, target].filter(Boolean).join(" · ");
}

function friendlyResourceContext(value: string) {
  const safeValue = safeResourceValue(value, false);
  if (!safeValue) return "";
  return safeValue;
}

function readableToolContext(value: string) {
  const [server = "", ...toolParts] = value.split("/");
  const tool = toolParts.join("/");
  return [server, tool]
    .map((part) => readableIdentifier(part))
    .filter(Boolean)
    .join(" · ");
}

function readableIdentifier(value: string) {
  return safeDisplayText(value)
    .replace(/[_:.-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function safeDisplayText(value: string, maximumLength = 320) {
  const normalized = value
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return normalized.length > maximumLength
    ? `${normalized.slice(0, maximumLength - 1)}…`
    : normalized;
}

const CONNECTOR_ACTIONS = new Set([
  "gmail.draft", "calendar.create", "calendar.update", "drive.export", "slack.post",
  "outlook.mail.search", "outlook.mail.read", "outlook.mail.draft",
  "outlook.calendar.read", "outlook.calendar.draft_event",
  "onedrive.file.search", "onedrive.file.read", "onedrive.file.write",
  "sharepoint.file.search", "sharepoint.file.read", "sharepoint.file.write",
  "teams.chat.search", "teams.chat.draft_message",
]);

const CONNECTOR_ACTION_LABEL_KEYS: Record<string, string> = {
  "gmail.draft": "integrations.scopes.create_gmail_drafts",
  "calendar.create": "chat.shield.connector_actions.calendar_create",
  "calendar.update": "chat.shield.connector_actions.calendar_update",
  "drive.export": "chat.shield.connector_actions.drive_export",
  "slack.post": "chat.shield.connector_actions.slack_post",
};

function connectorActionLabel(action: string, t: (key: string) => string) {
  const connectorKey = CONNECTOR_ACTION_LABEL_KEYS[action];
  if (connectorKey) return t(connectorKey);
  const key = CONNECTOR_ACTIONS.has(action) ? action.replaceAll(".", "_") : "unknown";
  return t(`microsoft365.capabilities.${key}`);
}

const APP_CONTROL_APPS = new Set(["Finder", "Preview", "Mail", "Calendar", "Numbers", "Keynote", "Excel", "PowerPoint"]);
const APP_CONTROL_ACTIONS = new Set(["focus", "press", "select", "type_text", "invoke_menu", "scroll", "drag_drop", "choose_file", "apple_event"]);

function safeAppControlPreview(preview: string) {
  try {
    const value = JSON.parse(preview) as Record<string, unknown>;
    if (!value || Array.isArray(value) || !APP_CONTROL_APPS.has(String(value.appName)) || !APP_CONTROL_ACTIONS.has(String(value.actionKind))) return null;
    return { appName: String(value.appName), actionKind: String(value.actionKind) };
  } catch {
    return null;
  }
}

type SemanticDetailRow = {
  label: string;
  value: string;
  verified: boolean;
};

function safeConnectorPreview(
  preview: string,
  operation: string,
  label: string,
  t: (key: string) => string,
) {
  if (!preview.trim()) return { rows: [], verified: false };
  try {
    const value = JSON.parse(preview) as unknown;
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return { rows: [], verified: false };
    }
    const rows = connectorDetailRows(
      sanitizeConnectorValue(
        semanticConnectorPreview(value as Record<string, unknown>, operation),
        operation,
        label,
      ),
      t,
    ).slice(0, 64);
    return {
      rows,
      verified: rows.length > 0 && rows.every((row) => row.verified),
    };
  } catch {
    return { rows: [], verified: false };
  }
}

const CONNECTOR_DATA_CLASSES: Record<string, string> = {
  search_query: "search_query", message_metadata: "message_metadata", message_content: "message_content",
  draft_recipients: "draft_recipients", draft_content: "draft_content", calendar_events: "calendar_events",
  event_details: "event_details", file_metadata: "file_metadata", file_content: "file_content",
  file_destination: "file_destination", site_identifier: "site_identifier", chat_messages: "chat_messages",
  site_url: "site_identifier", site_metadata: "site_identifier", chat_metadata: "chat_destination",
  chat_destination: "chat_destination",
};

function safeTransmissionPreview(preview: string, expectedDestination: string, t: (key: string) => string) {
  try {
    const value = JSON.parse(preview) as unknown;
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const record = value as Record<string, unknown>;
    if (typeof record.destination !== "string" || record.destination !== expectedDestination || !Array.isArray(record.dataClasses)) return null;
    const classes = [...new Set(record.dataClasses.filter((item): item is string => typeof item === "string").slice(0, 32))];
    if (!classes.length) return null;
    return {
      destination: safeApprovalDetailResource(record.destination, t),
      dataLabels: classes.map((item) => t(`chat.shield.connector_data.${CONNECTOR_DATA_CLASSES[item] ?? "other"}`)),
    };
  } catch {
    return null;
  }
}

function connectorDetailRows(value: unknown, t: (key: string) => string, prefix = ""): SemanticDetailRow[] {
  if (value === null || value === undefined) return [];
  if (["string", "number", "boolean"].includes(typeof value)) {
    const safeValue = safeConnectorDisplayValue(String(value), t);
    return [{
      label: prefix || "—",
      value: safeValue.value,
      verified: safeValue.verified,
    }];
  }
  if (Array.isArray(value)) {
    return value.flatMap((item, index) => connectorDetailRows(item, t, `${prefix || t("chat.shield.connector_fields.unknown")} · ${index + 1}`));
  }
  if (typeof value !== "object") return [];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, item]) => {
    const field = connectorFieldLabel(key, t);
    return connectorDetailRows(item, t, prefix ? `${prefix} · ${field}` : field);
  });
}

function safeConnectorDisplayValue(
  value: string,
  t: (key: string) => string,
) {
  const normalized = safeDisplayText(value, 1_000);
  if (
    /```|<\/?(?:script|style|iframe)\b|(?:\b(?:function|class|const|var|import|export)\b[^\n]{0,160}(?:=>|[{};]))/i.test(
      normalized,
    )
  ) {
    return { value: t("common.unknown"), verified: false };
  }
  const plain = normalized
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/(?:\*\*|__|~~)/g, "")
    .replace(/(?:^|\s)#{1,6}\s+/g, " ")
    .replace(/<[^>]{1,200}>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return plain
    ? { value: plain, verified: true }
    : { value: t("common.unknown"), verified: false };
}

const CONNECTOR_FIELDS: Record<string, string> = {
  to: "to", cc: "cc", subject: "subject", body: "body", path: "path", siteId: "site_id",
  destination: "destination", start: "start", end: "end",
  contentBytes: "content_bytes", contentType: "content_type", replaceExisting: "replace_existing",
};

function connectorFieldLabel(field: string, t: (key: string) => string) {
  return t(`chat.shield.connector_fields.${CONNECTOR_FIELDS[field] ?? "unknown"}`);
}

function semanticConnectorPreview(
  value: Record<string, unknown>,
  operation: string,
): Record<string, unknown> {
  if (operation === "slack.post") {
    return { destination: value.channel, body: value.text };
  }
  if (operation === "drive.export") {
    return { path: value.defaultFileName };
  }
  if (operation === "calendar.create" || operation === "calendar.update") {
    const event = operation === "calendar.update" && isRecord(value.event)
      ? value.event
      : value;
    return {
      subject: event.summary,
      body: event.description,
      destination: event.location,
      start: calendarMoment(event.start),
      end: calendarMoment(event.end),
      to: Array.isArray(event.attendees)
        ? event.attendees.flatMap((attendee) =>
          isRecord(attendee) && typeof attendee.email === "string"
            ? [attendee.email]
            : []
        )
        : undefined,
    };
  }
  return value;
}

function calendarMoment(value: unknown) {
  if (typeof value === "string") return value;
  if (!isRecord(value)) return undefined;
  const moment = typeof value.dateTime === "string"
    ? value.dateTime
    : typeof value.date === "string"
      ? value.date
      : "";
  const timeZone = typeof value.timeZone === "string" ? value.timeZone : "";
  return [moment, timeZone].filter(Boolean).join(" ") || undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function sanitizeConnectorValue(value: unknown, operation: string, label: string): unknown {
  if (typeof value === "string") return value === operation ? label : value;
  if (Array.isArray(value)) return value.slice(0, 32).map((item) => sanitizeConnectorValue(item, operation, label));
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(Object.entries(value as Record<string, unknown>)
    .filter(([key]) => key in CONNECTOR_FIELDS)
    .map(([key, item]) => [key, sanitizeConnectorValue(item, operation, label)]));
}

function shieldRiskLabel(riskTier: string, t: (key: string) => string) {
  const normalized = riskTier.toLowerCase();
  const key = ["file_read", "file_write", "system_exec", "low", "medium", "high"].includes(normalized) ? normalized : "unknown";
  return t(`chat.shield.risk_${key}`);
}

function shieldApprovalTierLabel(approvalTier: string, t: (key: string) => string) {
  switch (approvalTier) {
    case "background_auto_approval":
    case "background":
      return t("chat.shield.tier_background");
    case "visual_consent":
    case "visual":
      return t("chat.shield.tier_visual");
    case "explicit_confirmation":
    case "explicit":
      return t("chat.shield.tier_explicit");
    default:
      return t("chat.shield.tier_unknown");
  }
}

function formatApprovalTime(value: number, t: (key: string) => string) {
  if (!value) {
    return t("chat.shield.now");
  }
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(value));
}
