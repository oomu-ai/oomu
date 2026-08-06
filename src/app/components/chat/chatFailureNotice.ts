import { safeErrorMessage } from "@/lib/redaction";
import enUS from "@/locales/en-US.json";
import { chatErrorGroup } from "./chatErrorGroups";

type ChatFailureNotice = {
  content: string;
  status: string;
};

export type ChatTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

const DIRECT_NOTICE_KEYS: Record<string, [string, string]> = {
  permission_denied: ["permissions.permission_denied_content", "permissions.permission_denied_status"],
  permission_request: ["permissions.permission_request_content", "permissions.permission_request_status"],
  approved_file: ["permissions.approved_file_content", "permissions.approved_file_status"],
  approved_file_limit: ["permissions.approved_file_limit_content", "permissions.approved_file_limit_status"],
  contextual_file_preparation: ["permissions.contextual_file_preparation_content", "permissions.contextual_file_preparation_status"],
  private_egress: ["chat.private_egress_error.content", "chat.private_egress_error.status"],
  decision_pack_calendar_required: ["chat.errors.decision_pack_calendar_required.content", "chat.errors.decision_pack_calendar_required.status"],
};

// Resolve bundled English copy for test callers and non-hook code. Component
// callers pass the live translator so notices follow the active locale.
export function chatErrorFallbackTranslate(
  key: string,
  variables?: Record<string, string | number>,
) {
  const resolve = (root: unknown) => {
    let node = root;
    for (const part of key.split(".")) {
      if (!node || typeof node !== "object") return undefined;
      node = (node as Record<string, unknown>)[part];
    }
    return typeof node === "string" ? node : undefined;
  };
  let value = resolve(enUS) ?? key;
  Object.entries(variables ?? {}).forEach(([name, replacement]) => {
    value = value.split(`{${name}}`).join(String(replacement));
  });
  return value;
}

export function chatFailureNotice(
  error: unknown,
  t: ChatTranslate = chatErrorFallbackTranslate,
): ChatFailureNotice {
  let code = "";
  let detail = safeErrorMessage(error, "");

  if (typeof error === "string") {
    try {
      const parsed = JSON.parse(error) as { code?: unknown; message?: unknown };
      code = typeof parsed.code === "string" ? parsed.code : "";
      detail = safeErrorMessage(parsed, detail);
    } catch {
      // Tauri may return a plain string for non-structured command failures.
    }
  } else if (error && typeof error === "object" && "code" in error) {
    code = typeof error.code === "string" ? error.code : "";
  }

  const errorCode = /^[a-z][a-z0-9_]{0,79}$/i.test(code)
    ? code
    : "chat_request_failed";
  const group = chatErrorGroup(errorCode, detail);
  const directNoticeKey = DIRECT_NOTICE_KEYS[group];
  if (directNoticeKey) {
    return {
      content: t(directNoticeKey[0]),
      status: t(directNoticeKey[1]),
    };
  }
  if (group === "file_creation") {
    return {
      content: t("permissions.file_creation_failed_content"),
      status: t("permissions.file_creation_failed_status"),
    };
  }
  if (group === "final_verification") {
    return {
      content: t("chat.errors.final_verification.content"),
      status: t("chat.errors.final_verification.status"),
    };
  }
  if (group === "contextual_filename") {
    return {
      content: t("permissions.contextual_filename_question"),
      status: t("chat.status.ready"),
    };
  }
  if (group === "auto_route_attention") {
    return {
      content: t("chat.auto_route_attention.attention_content"),
      status: t("chat.route.needs_attention"),
    };
  }
  const exposeTechnicalDetails = ![
    "provider_network",
    "provider_rate_limited",
    "provider_response",
    "secure_memory",
    "turn_in_progress",
    "turn_persistence",
    "planner_unavailable",
    "planner_too_large",
    "local_action_unavailable",
    "connector_authority",
    "project_provider_blocked",
    "project_provider_consent",
    "private_egress",
    "contextual_output",
    "external_file_write",
    "delete_target_not_found",
  ].includes(group);
  const detailLine =
    exposeTechnicalDetails && detail ? `\n\n${t("chat.errors.details_line", { detail })}` : "";
  const codeLine = exposeTechnicalDetails
    ? `\n\n${t("chat.errors.code_line", { code: errorCode })}`
    : "";

  return {
    content: `${t(`chat.errors.${group}.content`)}${detailLine}${codeLine}`,
    status: t(`chat.errors.${group}.status`),
  };
}

export function localizePersistedAgentExecutionReceipt(
  content: string,
  t: ChatTranslate = chatErrorFallbackTranslate,
) {
  if (
    content.startsWith("Recovery Loop\n") &&
    content.includes("\nBoundary: MlcVerifier\n")
  ) {
    return t("chat.errors.final_verification.content");
  }
  if (
    content.startsWith("Recovery Loop\n") ||
    content ===
      "OOMU stopped before it could confirm the result. Check the result, then try again."
  ) {
    return t("chat.execution.stopped");
  }
  return content;
}
