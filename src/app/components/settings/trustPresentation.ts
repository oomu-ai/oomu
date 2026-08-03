export type TrustTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

const SCOPE_LABEL_KEYS: Record<string, string> = {
  once: "settings.privacy.trust.scope_once",
  app_session: "permissions.scope_app_session",
  task: "settings.privacy.trust.scope_task",
  project_path: "settings.privacy.trust.scope_project_path",
  persistent: "settings.privacy.trust.scope_persistent",
};

const ACTION_LABEL_KEYS: Record<string, string> = {
  filesystem_read: "settings.privacy.trust.action_read_files",
  filesystem_write: "settings.privacy.trust.action_change_files",
  file_read: "settings.privacy.trust.action_read_files",
  file_write: "settings.privacy.trust.action_change_files",
  codebase_patch: "settings.privacy.trust.action_change_code",
  document_index: "settings.privacy.trust.action_index_documents",
  delete_file: "settings.privacy.trust.action_delete_files",
  trash: "settings.privacy.trust.action_delete_files",
  trash_file: "settings.privacy.trust.action_delete_files",
  shell_command: "settings.privacy.trust.action_run_command",
  execute_command: "settings.privacy.trust.action_run_command",
  codebase_compile: "settings.privacy.trust.action_run_command",
  system_audit: "settings.privacy.trust.action_check_system",
  web_fetch: "settings.privacy.trust.action_use_network",
  network_request: "settings.privacy.trust.action_use_network",
  network_diagnostic: "settings.privacy.trust.action_check_network",
  mcp_connect_server: "settings.privacy.trust.action_connected_tool",
  mcp_execute_remote_tool: "settings.privacy.trust.action_connected_tool",
  connector_write: "settings.privacy.trust.action_change_service",
  connector_transmission: "settings.privacy.trust.action_share_with_service",
  app_control: "settings.privacy.trust.action_control_app",
  airlock_export: "settings.privacy.trust.action_export",
  telemetry_archive: "settings.privacy.trust.action_export",
  artifact_export: "settings.privacy.trust.action_export",
  workbook_export: "settings.privacy.trust.action_export",
  presentation_export: "settings.privacy.trust.action_export",
  approval_grant: "settings.privacy.trust.action_save_approval",
};

const STATUS_LABEL_KEYS: Record<string, string> = {
  approved: "settings.privacy.trust.status_approved",
  running: "settings.privacy.trust.status_running",
  completed: "settings.privacy.trust.status_completed",
  failed: "settings.privacy.trust.status_failed",
  recoverable: "settings.privacy.trust.status_recoverable",
  actuation_lease_paused: "settings.privacy.trust.status_paused",
  actuation_lease_unavailable: "settings.privacy.trust.status_unavailable",
  sensor_captured: "settings.privacy.trust.status_checked",
  blocked: "settings.privacy.trust.status_blocked",
  rejected: "settings.privacy.trust.status_rejected",
  cancelled: "settings.privacy.trust.status_cancelled",
  pending: "settings.privacy.trust.status_pending",
};

export function formatTrustScopeKind(value: string, t: TrustTranslate) {
  return t(
    SCOPE_LABEL_KEYS[value.trim().toLowerCase()] ??
      "settings.privacy.trust.scope_other",
  );
}

export function formatTrustAction(value: string, t: TrustTranslate) {
  const normalized = value.trim().replaceAll("-", "_").toLowerCase();
  if (normalized.startsWith("browser_")) {
    return t("settings.privacy.trust.action_browser");
  }
  return t(
    ACTION_LABEL_KEYS[normalized] ?? "settings.privacy.trust.action_other",
  );
}

export function formatTrustStatus(value: string, t: TrustTranslate) {
  return t(
    STATUS_LABEL_KEYS[value.trim().toLowerCase()] ??
      "settings.privacy.trust.status_unknown",
  );
}

const UNTIL_REVOKED_EXPIRY_THRESHOLD_MS = Date.UTC(9990, 0, 1);

export function isUntilRevokedReviewedScope(scope: {
  active?: boolean;
  scopeKind: string;
  expiresAtMs: number;
  revokedAtMs?: number | null;
}) {
  return (
    scope.active !== false &&
    scope.revokedAtMs == null &&
    scope.scopeKind === "persistent" &&
    scope.expiresAtMs >= UNTIL_REVOKED_EXPIRY_THRESHOLD_MS
  );
}
