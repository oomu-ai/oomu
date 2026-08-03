// Permission prompts are safety controls, not ordinary product copy. Every
// supported locale must have complete, honest wording here; falling back to
// English can make a high-stakes choice harder to understand.
import { readFileSync, readdirSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const localesDir = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "locales");
const referenceFile = "en-US.json";

const SHIELD_KEYS = [
  "paused",
  "configure_channel_detail", "configure_channel_reason",
  "connector_write_title", "connector_write_detail", "connector_write_reason", "connector_write_details",
  "connector_transmission_action", "connector_transmission_title", "connector_transmission_detail",
  "connector_transmission_reason", "connector_transmission_details", "connector_transmission_unavailable",
  "connector_data_included",
  "document_export_action", "document_export_title", "document_export_detail",
  "spreadsheet_export_action", "spreadsheet_export_title", "spreadsheet_export_detail",
  "presentation_export_action", "presentation_export_title", "presentation_export_detail",
  "export_reason",
  "app_control_action", "app_control_title", "app_control_detail", "app_control_reason",
  "app_control_unknown_app", "app_control_unavailable", "action_unavailable",
  "scope_title", "scope_task", "scope_project_path", "scope_unknown", "always_confirm",
  "risk_file_read", "risk_file_write", "risk_system_exec", "risk_low", "risk_medium", "risk_high", "risk_unknown",
  "tier_background", "tier_visual", "tier_explicit", "tier_unknown",
  "now", "category", "approval", "requested", "deny", "approve", "resolving",
].map((key) => `chat.shield.${key}`);

const MICROSOFT_KEYS = [
  "product_name", "current_consent_help", "consent_review_help", "current_consent",
  "consent_review_title", "continue_to_microsoft", "no_remote_scope",
  "no_destination_reported", "exact_scopes", "data_destinations", "consent_not_approval",
].map((key) => `microsoft365.${key}`);

const MICROSOFT_LABEL_KEYS = [
  "what_this_allows", "connects_to", "technical_details", "scope_identify_account",
  "scope_keep_connected", "scope_read_mail", "scope_prepare_mail", "scope_read_calendar",
  "scope_prepare_calendar", "scope_read_onedrive", "scope_update_onedrive",
  "scope_read_sharepoint", "scope_update_sharepoint", "scope_read_teams",
  "scope_prepare_teams", "scope_other", "destination_sign_in",
  "destination_services", "destination_other",
].map((key) => `microsoft365_labels.${key}`);

const GENERIC_ACTION_KEYS = [
  "action_save_approval", "action_delete_files", "action_run_command", "action_use_network",
  "action_check_network", "action_connected_tool", "action_check_system", "action_export",
  "action_browser", "action_other",
].map((key) => `settings.privacy.trust.${key}`);

const FIXED_KEYS = [
  "common.cancel", "common.close", "common.details",
  "integration_actions.opening_service",
  "app_control_actions.activate_approval",
  "trust.configure_channel_prompt", "trust.configure_channel_prompt_no_owner",
  "tools.configure_channel.activate", "tools.configure_channel.deactivate",
  "integrations.review_access", "integrations.consent_title", "integrations.consent_help",
  "integrations.exact_access", "integrations.continue_to_service", "integrations.destinations",
  "workflows.library.approve_step", "workflows.library.approving", "workflows.library.approve",
  "mods.capability_sentences.other",
  ...SHIELD_KEYS,
  ...MICROSOFT_KEYS,
  ...MICROSOFT_LABEL_KEYS,
  ...GENERIC_ACTION_KEYS,
];

const LOCALIZED_SHARED_TERM_ALLOWLIST = new Set([
  "microsoft365.product_name",
  "integrations.service_names.google_workspace",
  "integrations.service_names.slack",
]);

export function valueAtPath(value, path) {
  return path.split(".").reduce((node, key) => {
    if (!node || typeof node !== "object") return undefined;
    return node[key];
  }, value);
}

function leafPaths(value, prefix) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  return Object.entries(value).flatMap(([key, child]) => {
    const path = `${prefix}.${key}`;
    return child && typeof child === "object" && !Array.isArray(child)
      ? leafPaths(child, path)
      : [path];
  });
}

export function permissionSurfaceKeys(reference) {
  const prefixes = [
    "permissions",
    "mcp_confirmation",
    "chat.shield.connector_data",
    "chat.shield.connector_actions",
    "chat.shield.app_control_actions",
    "chat.shield.connector_fields",
    "integrations.service_names",
    "integrations.scopes",
    "microsoft365.capabilities",
    "sprint_301",
  ];
  return [...new Set([
    ...FIXED_KEYS,
    ...prefixes.flatMap((prefix) => leafPaths(valueAtPath(reference, prefix), prefix)),
  ])].sort();
}

function placeholders(value) {
  if (typeof value !== "string") return [];
  return [...new Set(value.match(/\{[A-Za-z0-9_]+\}/g) ?? [])].sort();
}

export function permissionSurfaceIssuesForLocale(data, reference, file) {
  const issues = [];
  for (const path of permissionSurfaceKeys(reference)) {
    const value = valueAtPath(data, path);
    const referenceValue = valueAtPath(reference, path);
    if (typeof value !== "string" || value.trim().length === 0) {
      issues.push(`${path}: missing or empty`);
      continue;
    }
    if (placeholders(value).join("|") !== placeholders(referenceValue).join("|")) {
      issues.push(`${path}: placeholders do not match en-US`);
    }
    if (
      file !== referenceFile &&
      value.trim() === String(referenceValue).trim() &&
      !LOCALIZED_SHARED_TERM_ALLOWLIST.has(path)
    ) {
      issues.push(`${path}: untranslated English copy`);
    }
  }
  return issues;
}

export function runPermissionSurfaceLocaleCheck() {
  const reference = JSON.parse(readFileSync(join(localesDir, referenceFile), "utf8"));
  const files = readdirSync(localesDir).filter(
    (file) => file.endsWith(".json") && file !== referenceFile,
  );
  let failures = 0;
  for (const file of files) {
    const data = JSON.parse(readFileSync(join(localesDir, file), "utf8"));
    const issues = permissionSurfaceIssuesForLocale(data, reference, file);
    failures += issues.length;
    if (issues.length) {
      console.error(`✗ ${file} has ${issues.length} permission localization issue(s):`);
      issues.forEach((issue) => console.error(`    ${issue}`));
    } else {
      console.log(`✓ ${file} permission copy`);
    }
  }
  if (failures) {
    console.error(`\nPermission locale check failed with ${failures} issue(s).`);
  } else {
    console.log("\nEvery permission surface is complete in every locale.");
  }
  return failures;
}

function isInvokedDirectly() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isInvokedDirectly()) {
  process.exit(runPermissionSurfaceLocaleCheck() > 0 ? 1 : 0);
}
