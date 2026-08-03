import {
  localizeCapabilityAction,
  type CapabilityCatalog,
  type MissingCapabilityDetail,
  type WorkflowCapabilityAction,
} from "./workflowCapabilityCatalog";

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export const TOPOLOGY_MISSING_REPORT_WRITER_CODE =
  "workflow_topological_anomaly_missing_report_writer";
const TOPOLOGY_UNSAFE_COLLECTION_CODE =
  "workflow_topological_anomaly_unsafe_collection_access";

type ComposerNoticeTone = "info" | "success" | "warning" | "error";

export type NoticeCapability = {
  id?: string;
  title: string;
  outcome?: string;
  reason?: string;
  source?: string;
  serverName?: string;
  toolName?: string;
};

export function normalizeNoticeCapabilities(
  missingCapabilities: string[] | undefined,
  missingCapabilityDetails: MissingCapabilityDetail[] | undefined,
): NoticeCapability[] {
  const capabilities: NoticeCapability[] = [];
  const seen = new Set<string>();
  const addCapability = (capability: NoticeCapability) => {
    const keys = [capability.id, capability.title]
      .filter((value): value is string => Boolean(value))
      .map(normalizeCapabilityKey);
    if (keys.some((key) => seen.has(key))) return;
    keys.forEach((key) => seen.add(key));
    capabilities.push(capability);
  };
  for (const detail of missingCapabilityDetails ?? []) addCapability(detail);
  for (const capability of missingCapabilities ?? []) {
    addCapability({
      id: capability,
      title: capability,
      outcome: capability,
      reason: capability,
    });
  }
  return capabilities;
}

export function localizeMissingCapabilityDetails(
  details: MissingCapabilityDetail[] | undefined,
  catalog: CapabilityCatalog | null,
  t: TranslateFn,
) {
  return details?.map((detail) => {
    const action = findCatalogActionForCapability(detail, catalog);
    if (!action) return detail;
    const localized = localizeCapabilityAction(action, t);
    return { ...detail, outcome: localized.outcome, title: localized.title };
  });
}

export function localizeMissingCapabilities(
  missingCapabilities: string[] | undefined,
  catalog: CapabilityCatalog | null,
  t: TranslateFn,
) {
  return missingCapabilities?.map((capability) => {
    const action = findCatalogActionForCapability(
      { id: capability, title: capability },
      catalog,
    );
    return action ? localizeCapabilityAction(action, t).title : capability;
  });
}

function findCatalogActionForCapability(
  capability: Pick<NoticeCapability, "id" | "serverName" | "title" | "toolName">,
  catalog: CapabilityCatalog | null,
) {
  return catalog?.actions.find((action) => capabilityMatchesAction(action, capability));
}

export function capabilityMatchesAction(
  action: WorkflowCapabilityAction,
  capability: NoticeCapability,
) {
  const capabilityKeys = [
    capability.id,
    capability.title,
    capability.serverName && capability.toolName
      ? `mcp:${capability.serverName}:${capability.toolName}`
      : undefined,
  ]
    .filter((value): value is string => Boolean(value))
    .map(normalizeCapabilityKey);
  const actionKeys = [
    action.id,
    action.title,
    action.serverName && action.toolName
      ? `mcp:${action.serverName}:${action.toolName}`
      : undefined,
  ]
    .filter((value): value is string => Boolean(value))
    .map(normalizeCapabilityKey);
  return actionKeys.some((key) => capabilityKeys.includes(key));
}

function normalizeCapabilityKey(value: string) {
  return value.trim().toLowerCase();
}

export function compilerErrorCode(error: unknown) {
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.code === "string" && record.code.trim()) {
      return record.code.trim();
    }
    const detail = record.detail;
    if (detail && typeof detail === "object") {
      const detailRecord = detail as Record<string, unknown>;
      if (typeof detailRecord.code === "string" && detailRecord.code.trim()) {
        return detailRecord.code.trim();
      }
    }
  }

  const message = extractErrorMessage(error);
  return message.match(/\(([a-z][a-z0-9_]*?)\)\s*$/i)?.[1];
}

export function friendlyAuthoringError(error: unknown, t: TranslateFn) {
  if (compilerErrorCode(error) === TOPOLOGY_UNSAFE_COLLECTION_CODE) {
    return t("workflows.composer.collection_check_error");
  }
  if (compilerErrorCode(error) === "workflow_composer_timeout") {
    return t("workflows.composer.timeout_error");
  }
  const message = extractErrorMessage(error);
  if (!message) return t("workflows.composer.unknown_error");

  const normalized = message.toLowerCase();
  if (normalized.includes("workflow_composer_timeout")) {
    return t("workflows.composer.timeout_error");
  }
  if (
    normalized.includes("zoderror") ||
    normalized.includes("invalid_type") ||
    normalized.includes("workflow_ir_invalid") ||
    normalized.includes("workflow ir") ||
    normalized.includes("schema")
  ) {
    return t("workflows.composer.validation_error");
  }
  if (
    normalized.includes("failed to fetch") ||
    normalized.includes("networkerror") ||
    normalized.includes("connection refused")
  ) {
    return t("workflows.composer.connection_error");
  }
  return message;
}

function extractErrorMessage(error: unknown) {
  if (typeof error === "string") return error.trim();
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim()) {
      return record.message.trim();
    }
    if (typeof record.reason === "string" && record.reason.trim()) {
      return record.reason.trim();
    }
  }
  return "";
}

export function composerNoticeClasses(tone: ComposerNoticeTone) {
  if (tone === "success") {
    return "border-[var(--success)]/30 bg-[var(--success-background)] text-[var(--foreground)]";
  }
  if (tone === "warning") {
    return "border-[var(--warning)]/30 bg-[var(--warning-background)] text-[var(--foreground)]";
  }
  if (tone === "error") {
    return "border-[var(--destructive)]/30 bg-[var(--destructive-background)] text-[var(--destructive)]";
  }
  return "border-[var(--border-soft)] bg-[var(--accent-background)] text-[var(--foreground)]";
}
