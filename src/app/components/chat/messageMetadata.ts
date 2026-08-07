import { chatFailureNotice, type ChatTranslate } from "./chatFailureNotice";

export type PublicGroundingProvenance = {
  url: string;
  accessedAtUtc: string;
};

export type PermissionContinuationMetadata = {
  state: "waiting" | "retrying" | "completed";
  capabilityId: string;
  errorCode?: string;
  boundary?: string;
};

export type ChatMessageMetadata = {
  turnId?: string;
  rootTurnId?: string;
  generationToken?: string;
  turnState?: string;
  routingMode?: string;
  eventKind?: string;
  secureMemoryStatus?: string;
  verifiedNativeExecutionReceipt?: boolean;
  executingProviderId?: string;
  executingModelId?: string;
  targetProviderId?: string;
  targetModelId?: string;
  terminalResultForTurnId?: string;
  terminalErrorCode?: string;
  terminalErrorBoundary?: string;
  checkpointForTurnId?: string;
  checkpointKind?: string;
  permissionRestoredForTurnId?: string;
  nativeReceiptId?: string;
  capabilityId?: string;
  localizationKey?: string;
  uiOnlyCheckpoint?: boolean;
  publicGroundingProvenance?: PublicGroundingProvenance[];
  finishReason?: string;
  contextCondensed?: boolean;
  contextBudgetTokens?: number;
  contextSourcesPreserved?: boolean;
  permissionContinuation?: PermissionContinuationMetadata;
};

export function isInternalUiOnlyCheckpoint(metadata: ChatMessageMetadata | null) {
  return metadata?.uiOnlyCheckpoint === true && (
    metadata.checkpointKind === "sovereign_search_progress" ||
    metadata.checkpointKind === "verified_sovereign_search"
  );
}

const uiCheckpointLocalizationKeys = new Set([
  "chat.search_errors.ambient_unavailable",
  "chat.search_fallback.started",
  "chat.search_fallback.failed",
  "sprint_301.permission_recovery.cancelled",
  "sprint_301.permission_recovery.restored",
]);

export function localizedUiCheckpointContent(
  metadata: ChatMessageMetadata | null,
  translate: ChatTranslate,
) {
  const localizationKey = metadata?.localizationKey;
  if (localizationKey === "sprint_301.permission_recovery.restored" &&
    metadata?.checkpointKind === "permission_recovery_restored" && metadata.capabilityId &&
    metadata.nativeReceiptId) {
    return translate(localizationKey, { capability: translate(
      `sprint_299.permissions.capabilities.${metadata.capabilityId}.name`,
    ) });
  }
  return metadata?.uiOnlyCheckpoint === true && localizationKey &&
      uiCheckpointLocalizationKeys.has(localizationKey)
    ? translate(localizationKey)
    : null;
}

export function permissionRestoredPresentation(metadata: ChatMessageMetadata | null | undefined) {
  if (metadata?.checkpointKind !== "permission_recovery_restored" || !metadata.nativeReceiptId) {
    return null;
  }
  return {
    attributes: {
      "aria-live": "polite" as const,
      "data-native-receipt-id": metadata.nativeReceiptId,
      "data-permission-recovery-state": "restored",
      role: "status" as const,
    },
    bubbleClassName: "self-start rounded-[var(--radius-lg)] border border-[var(--success)]/30 bg-[var(--success-background)] text-[var(--foreground)]",
  };
}

export function localizedAssistantTerminalContent(
  content: string,
  metadata: ChatMessageMetadata | null,
  translate: ChatTranslate,
) {
  if (metadata?.terminalErrorCode) {
    return chatFailureNotice({ code: metadata.terminalErrorCode }, translate).content;
  }
  return content.trim() === "search_incomplete"
    ? translate("chat.search_errors.search_incomplete")
    : content;
}

export function localizedAssistantResponse(
  text: string,
  rawMetadata: unknown,
  providerId: string,
  modelId: string,
  translate: ChatTranslate,
) {
  const metadata = normalizeChatMessageMetadata(rawMetadata, providerId, modelId);
  return {
    metadata,
    text: localizedAssistantTerminalContent(sanitizeInferenceText(text), metadata, translate),
  };
}

function metadataString(metadata: Record<string, unknown>, ...keys: string[]) {
  return keys
    .map((key) => metadata[key])
    .find((value): value is string => typeof value === "string" && value.trim().length > 0)
    ?.trim();
}

function metadataBoolean(metadata: Record<string, unknown>, ...keys: string[]) {
  return keys
    .map((key) => metadata[key])
    .find((value): value is boolean => typeof value === "boolean");
}

function metadataNumber(metadata: Record<string, unknown>, ...keys: string[]) {
  return keys
    .map((key) => metadata[key])
    .find((value): value is number => typeof value === "number" && Number.isFinite(value));
}

function permissionContinuation(
  metadata: Record<string, unknown>,
): PermissionContinuationMetadata | undefined {
  const raw = metadata.permissionContinuation ?? metadata.permission_continuation;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return undefined;
  const value = raw as Record<string, unknown>;
  const state = metadataString(value, "state");
  const capabilityId = metadataString(value, "capabilityId", "capability_id");
  if (!capabilityId || !["waiting", "retrying", "completed"].includes(state ?? "")) {
    return undefined;
  }
  return {
    capabilityId,
    state: state as PermissionContinuationMetadata["state"],
    errorCode: metadataString(value, "errorCode", "error_code"),
    boundary: metadataString(value, "boundary"),
  };
}

function publicGroundingProvenance(
  metadata: Record<string, unknown>,
): PublicGroundingProvenance[] | undefined {
  const raw = metadata.publicGroundingProvenance ?? metadata.public_grounding_provenance;
  if (!Array.isArray(raw)) return undefined;
  const seen = new Set<string>();
  const normalized = raw.flatMap((entry) => {
    if (!entry || typeof entry !== "object") return [];
    const value = entry as Record<string, unknown>;
    const url = metadataString(value, "url");
    const accessedAtUtc = metadataString(value, "accessedAtUtc", "accessed_at_utc");
    if (!url || !accessedAtUtc || !accessedAtUtc.endsWith("Z") || Number.isNaN(Date.parse(accessedAtUtc))) {
      return [];
    }
    try {
      const parsed = new URL(url);
      if (parsed.protocol !== "https:" || parsed.username || parsed.password || !parsed.hostname) {
        return [];
      }
    } catch {
      return [];
    }
    if (seen.has(url)) return [];
    seen.add(url);
    return [{ url, accessedAtUtc }];
  }).slice(0, 5);
  return normalized.length > 0 ? normalized : undefined;
}

export function normalizeChatMessageMetadata(
  raw: unknown,
  fallbackProviderId?: string | null,
  fallbackModelId?: string | null,
): ChatMessageMetadata | null {
  let value = raw;
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      value = null;
    }
  }
  const metadata = value && typeof value === "object"
    ? value as Record<string, unknown>
    : {};
  const normalized: ChatMessageMetadata = {
    turnId: metadataString(metadata, "turnId", "turn_id"),
    rootTurnId: metadataString(metadata, "rootTurnId", "root_turn_id"),
    generationToken: metadataString(metadata, "generationToken", "generation_token"),
    turnState: metadataString(metadata, "turnState", "turn_state"),
    routingMode: metadataString(metadata, "routingMode", "routing_mode"),
    eventKind: metadataString(metadata, "eventKind", "event_kind"),
    secureMemoryStatus: metadataString(metadata, "secureMemoryStatus", "secure_memory_status"),
    verifiedNativeExecutionReceipt: metadataBoolean(metadata, "verifiedNativeExecutionReceipt", "verified_native_execution_receipt"),
    executingProviderId: metadataString(metadata, "executingProviderId", "executing_provider_id") ?? fallbackProviderId ?? undefined,
    executingModelId: metadataString(metadata, "executingModelId", "executing_model_id") ?? fallbackModelId ?? undefined,
    targetProviderId: metadataString(metadata, "targetProviderId", "target_provider_id"),
    targetModelId: metadataString(metadata, "targetModelId", "target_model_id"),
    terminalResultForTurnId: metadataString(
      metadata,
      "terminalResultForTurnId",
      "terminal_result_for_turn_id",
    ),
    terminalErrorCode: metadataString(metadata, "terminalErrorCode", "terminal_error_code"),
    terminalErrorBoundary: metadataString(
      metadata,
      "terminalErrorBoundary",
      "terminal_error_boundary",
    ),
    checkpointForTurnId: metadataString(
      metadata,
      "checkpointForTurnId",
      "checkpoint_for_turn_id",
    ),
    checkpointKind: metadataString(metadata, "checkpointKind", "checkpoint_kind"),
    permissionRestoredForTurnId: metadataString(
      metadata,
      "permissionRestoredForTurnId",
      "permission_restored_for_turn_id",
    ),
    nativeReceiptId: metadataString(metadata, "nativeReceiptId", "native_receipt_id"),
    capabilityId: metadataString(metadata, "capabilityId", "capability_id"),
    localizationKey: metadataString(metadata, "localizationKey", "localization_key"),
    uiOnlyCheckpoint: metadataBoolean(metadata, "uiOnlyCheckpoint", "ui_only_checkpoint"),
    publicGroundingProvenance: publicGroundingProvenance(metadata),
    finishReason: metadataString(metadata, "finishReason", "finish_reason"),
    contextCondensed: metadataBoolean(metadata, "contextCondensed", "context_condensed"),
    contextBudgetTokens: metadataNumber(metadata, "contextBudgetTokens", "context_budget_tokens"),
    contextSourcesPreserved: metadataBoolean(
      metadata,
      "contextSourcesPreserved",
      "context_sources_preserved",
    ),
    permissionContinuation: permissionContinuation(metadata),
  };
  return Object.values(normalized).some((entry) => entry !== undefined) ? normalized : null;
}

export function markAcceptedTurnTerminal<T extends {
  role: string;
  isPending?: boolean;
  metadata?: ChatMessageMetadata | null;
}>(messages: T[], turnId: string, state: "completed" | "failed" | "cancelled") {
  return messages.map((message) =>
    message.role === "user" &&
    message.metadata?.turnId === turnId &&
    message.metadata.turnState === "accepted"
      ? { ...message, isPending: false, metadata: { ...message.metadata, turnState: state } }
      : message,
  );
}

export function markAcceptedTurnTerminalAfterError<T extends {
  role: string;
  metadata?: ChatMessageMetadata | null;
}>(messages: T[], turnId: string, errorCode: string) {
  if (errorCode === "chat_turn_already_running") return messages;
  const cancelled = [
    "local_inference_cancelled",
    "auto_route_choice_cancelled",
    "project_cloud_choice_cancelled",
    "private_egress_user_denied",
  ].includes(errorCode);
  return markAcceptedTurnTerminal(messages, turnId, cancelled ? "cancelled" : "failed");
}
import { sanitizeInferenceText } from "@/lib/InferenceService";
