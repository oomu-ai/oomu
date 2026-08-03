import type { ApprovalRequest } from "./workflowPersistence";

type TranslateFn = (key: string) => string;

export function approvalPreviewFromRequest(
  approval: ApprovalRequest,
  t: TranslateFn,
) {
  const context = asRecord(approval.context);
  const actionType = typeof context.actionType === "string" ? context.actionType : "";

  if (actionType === "mcp_tool") {
    const argumentsRecord = asRecord(context.arguments);
    const approvalReuse = asRecord(context.approvalReuse);
    const exactPreview = exactNativeMcpPreview(context, argumentsRecord, t);
    const serverName = context.serverName === "oomu_task_tools"
      ? "OOMU"
      : typeof context.serverName === "string"
        ? readableIdentifier(context.serverName)
        : null;
    const toolName = typeof context.toolName === "string"
      ? readableIdentifier(context.toolName)
      : null;
    return {
      canApprove: Boolean(
        serverName && toolName && (exactPreview?.canApprove ?? true),
      ),
      argumentsLabel: t("mcp_confirmation.arguments"),
      argumentsValue:
        exactPreview?.argumentsValue ??
        mcpSemanticSummary(context, argumentsRecord, t),
      serverLabel: t("mcp_confirmation.server"),
      serverName: serverName ?? t("settings.privacy.trust.action_connected_tool"),
      toolLabel: t("mcp_confirmation.tool"),
      toolName:
        exactPreview?.toolName ??
        toolName ??
        t("settings.privacy.trust.action_other"),
      reusableForWorkflowVersion:
        approvalReuse.scope === "workflow_version",
    };
  }

  if (actionType === "system_action") {
    const folder = friendlyBasename(context.workingDirectory);
    const mode = typeof context.mode === "string" ? context.mode : "";
    const riskTier = typeof context.capabilityRiskTier === "string"
      ? context.capabilityRiskTier
      : "";
    const actionLabelKey = SYSTEM_ACTION_LABEL_KEYS[riskTier];
    const canApprove = Boolean(
      actionLabelKey && ["binary", "python", "shell"].includes(mode),
    );
    return {
      canApprove,
      argumentsLabel: t("mcp_confirmation.arguments"),
      argumentsValue: folder
        ? [`${t("mcp_confirmation.location")}: ${folder}`]
        : [],
      serverLabel: t("mcp_confirmation.server"),
      serverName: t("workflows.trust.touches.local_mac"),
      toolLabel: t("mcp_confirmation.tool"),
      toolName: actionLabelKey
        ? t(actionLabelKey)
        : t("mcp_confirmation.local_action"),
      reusableForWorkflowVersion: false,
    };
  }

  if (actionType === "workflow_permission") {
    const permissionKind = typeof context.permissionKind === "string"
      ? context.permissionKind
      : "";
    const actionLabelKey = WORKFLOW_PERMISSION_LABEL_KEYS[permissionKind];
    const purpose = safePromptText(context.capabilityReason);
    const workflowLabel = safePromptText(context.actionLabel, 90);
    return {
      canApprove: Boolean(actionLabelKey),
      argumentsLabel: t("mcp_confirmation.arguments"),
      argumentsValue: [
        ...(workflowLabel
          ? [`${t("mcp_confirmation.purpose")}: ${workflowLabel}`]
          : []),
        ...(purpose && purpose !== workflowLabel
          ? [purpose]
          : []),
      ],
      serverLabel: t("mcp_confirmation.server"),
      serverName: "OOMU",
      toolLabel: t("mcp_confirmation.tool"),
      toolName: actionLabelKey
        ? t(actionLabelKey)
        : t("settings.privacy.trust.action_other"),
      reusableForWorkflowVersion: false,
    };
  }

  const fallbackAction = safePromptText(
    firstString(context.actionLabel, context.capabilityReason, context.purpose),
    90,
  );
  return {
    canApprove: false,
    argumentsLabel: t("mcp_confirmation.arguments"),
    argumentsValue: [],
    serverLabel: t("mcp_confirmation.server"),
    serverName: "OOMU",
    toolLabel: t("mcp_confirmation.tool"),
    toolName: fallbackAction ?? t("workflows.library.approve_step"),
    reusableForWorkflowVersion: false,
  };
}

// These labels are selected only from the backend's closed risk-tier enum.
// Model prose, commands, and arguments never become approval authority or UI.
const SYSTEM_ACTION_LABEL_KEYS: Record<string, string> = {
  READ_ONLY: "settings.privacy.trust.action_check_system",
  FILE_READ: "settings.privacy.trust.action_read_files",
  FILE_WRITE: "settings.privacy.trust.action_change_files",
  SYSTEM_EXEC: "mcp_confirmation.local_action",
  NETWORK: "settings.privacy.trust.action_use_network",
};

// Permission kinds are emitted by the backend's closed PermissionKind enum.
// Custom or future kinds stay disabled until their meaning is explicitly added.
const WORKFLOW_PERMISSION_LABEL_KEYS: Record<string, string> = {
  file_read: "settings.privacy.trust.action_read_files",
  file_write: "settings.privacy.trust.action_change_files",
  network: "settings.privacy.trust.action_use_network",
  process: "settings.privacy.trust.action_run_command",
  mcp_tool: "settings.privacy.trust.action_connected_tool",
};

type ExactNativeMcpPreview = {
  argumentsValue: string[];
  canApprove: boolean;
  toolName: string;
};

function exactNativeMcpPreview(
  context: Record<string, unknown>,
  argumentsRecord: Record<string, unknown>,
  t: TranslateFn,
): ExactNativeMcpPreview | null {
  if (context.serverName !== "oomu_task_tools") return null;

  if (
    context.toolName === "draft_system_email" ||
    context.toolName === "draft_decision_pack_email" ||
    context.toolName === "draft_release_recovery_email"
  ) {
    const to = optionalBoundedApprovalText(argumentsRecord.to, 4_096);
    const subject = boundedApprovalText(argumentsRecord.subject, 998);
    const cc = optionalBoundedApprovalText(argumentsRecord.cc, 4_096);
    const bcc = optionalBoundedApprovalText(argumentsRecord.bcc, 4_096);
    const recipientRequired = context.toolName !== "draft_system_email";
    return {
      canApprove: Boolean(
        subject &&
          to.valid &&
          (!recipientRequired || to.value) &&
          cc.valid &&
          bcc.valid,
      ),
      toolName: t("chat.recovery.approval_save_draft_only"),
      argumentsValue: [
        ...(to.value ? [`${t("mcp_confirmation.recipient")}: ${to.value}`] : []),
        ...(cc.value ? [`${t("mcp_confirmation.cc")}: ${cc.value}`] : []),
        ...(bcc.value ? [`${t("mcp_confirmation.bcc")}: ${bcc.value}`] : []),
        ...(subject ? [`${t("mcp_confirmation.subject")}: ${subject}`] : []),
      ],
    };
  }

  if (context.toolName === "send_system_email") {
    const to = boundedApprovalText(argumentsRecord.to, 4_096);
    const subject = boundedApprovalText(argumentsRecord.subject, 998);
    const cc = optionalBoundedApprovalText(argumentsRecord.cc, 4_096);
    const bcc = optionalBoundedApprovalText(argumentsRecord.bcc, 4_096);
    const attachment = optionalBoundedApprovalText(
      argumentsRecord.attachmentPath,
      4_096,
    );
    const attachmentName = attachment.value
      ? exactFileBasename(attachment.value)
      : null;
    return {
      canApprove: Boolean(
        to &&
          subject &&
          cc.valid &&
          bcc.valid &&
          attachment.valid &&
          (!attachment.value || attachmentName),
      ),
      toolName: t("mcp_confirmation.action_send_email"),
      argumentsValue: [
        ...(to ? [`${t("mcp_confirmation.recipient")}: ${to}`] : []),
        ...(cc.value ? [`${t("mcp_confirmation.cc")}: ${cc.value}`] : []),
        ...(bcc.value ? [`${t("mcp_confirmation.bcc")}: ${bcc.value}`] : []),
        ...(subject ? [`${t("mcp_confirmation.subject")}: ${subject}`] : []),
        ...(attachmentName
          ? [
              `${t("mcp_confirmation.attachment")}: ${attachmentName}`,
            ]
          : []),
      ],
    };
  }

  if (context.toolName === "create_conflict_free_calendar_event") {
    const calendar = boundedApprovalText(argumentsRecord.calendarName, 160);
    const eventTitle = boundedApprovalText(argumentsRecord.title, 240);
    const day = argumentsRecord.day === "next_weekday"
      ? t("mcp_confirmation.next_weekday")
      : null;
    const windowStart = boundedLocalTime(argumentsRecord.windowStartLocal);
    const windowEnd = boundedLocalTime(argumentsRecord.windowEndLocal);
    const duration = typeof argumentsRecord.durationMinutes === "number"
      && Number.isInteger(argumentsRecord.durationMinutes)
      && argumentsRecord.durationMinutes > 0
      && argumentsRecord.durationMinutes <= 24 * 60
      ? argumentsRecord.durationMinutes
      : null;
    return {
      canApprove: Boolean(
        calendar && eventTitle && day && windowStart && windowEnd && duration,
      ),
      toolName: t("mcp_confirmation.action_create_calendar_event"),
      argumentsValue: [
        ...(calendar
          ? [`${t("mcp_confirmation.calendar")}: ${calendar}`]
          : []),
        ...(eventTitle
          ? [`${t("mcp_confirmation.event_title")}: ${eventTitle}`]
          : []),
        ...(day && windowStart && windowEnd
          ? [
              `${t("mcp_confirmation.time_window")}: ${day}, ${windowStart}–${windowEnd}`,
            ]
          : []),
        ...(duration
          ? [
              `${t("mcp_confirmation.duration")}: ${duration} ${t("mcp_confirmation.minutes")}`,
            ]
          : []),
      ],
    };
  }

  return null;
}

function mcpSemanticSummary(
  context: Record<string, unknown>,
  argumentsRecord: Record<string, unknown>,
  t: TranslateFn,
) {
  const summary: string[] = [];
  const purpose = safePromptText(context.capabilityReason);
  if (purpose) {
    summary.push(`${t("mcp_confirmation.purpose")}: ${purpose}`);
  }

  const path = firstString(
    argumentsRecord.path,
    argumentsRecord.targetPath,
    argumentsRecord.target_path,
    argumentsRecord.destinationPath,
    argumentsRecord.destination_path,
    argumentsRecord.filePath,
    argumentsRecord.file_path,
    argumentsRecord.folder,
  );
  const destination =
    friendlyBasename(path) ??
    safePromptText(
      firstString(
        argumentsRecord.destination,
        argumentsRecord.channel,
        argumentsRecord.conversation,
        argumentsRecord.recipient,
        argumentsRecord.to,
      ),
    );
  if (destination) {
    summary.push(`${t("mcp_confirmation.destination")}: ${destination}`);
  }
  return summary;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function boundedApprovalText(value: unknown, maxLength: number) {
  if (typeof value !== "string" || value.length > maxLength) return null;
  if (/[\u0000-\u001F\u007F]/.test(value)) return null;
  const normalized = value.trim();
  return normalized || null;
}

function optionalBoundedApprovalText(value: unknown, maxLength: number) {
  if (value === undefined || value === null || value === "") {
    return { valid: true, value: null };
  }
  const bounded = boundedApprovalText(value, maxLength);
  return { valid: Boolean(bounded), value: bounded };
}

function boundedLocalTime(value: unknown) {
  return typeof value === "string" && /^(?:[01]\d|2[0-3]):[0-5]\d$/.test(value)
    ? value
    : null;
}

function readableIdentifier(value: string) {
  const readableValue = sanitizeTechnicalText(value)
    .replace(/[_:/.-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const cleaned = safePromptText(readableValue, 80) ?? "";

  return cleaned.length > 0 ? cleaned.slice(0, 80) : null;
}

function firstString(...values: unknown[]) {
  return values.find(
    (value): value is string => typeof value === "string" && value.trim().length > 0,
  );
}

function friendlyBasename(value: unknown) {
  if (typeof value !== "string" || /^https?:\/\//i.test(value)) return null;
  const basename = value.replace(/\\/g, "/").split("/").filter(Boolean).at(-1);
  if (!basename || basename === "." || basename === "..") return null;
  return safePromptText(basename, 80);
}

function exactFileBasename(value: unknown) {
  if (typeof value !== "string" || /^https?:\/\//i.test(value)) return null;
  const basename = value.replace(/\\/g, "/").split("/").filter(Boolean).at(-1);
  if (!basename || basename === "." || basename === "..") return null;
  if (/\b(?:bearer|api[ _-]?key|password|secret|token)\b/i.test(basename)) {
    return null;
  }
  if (/\b[a-f0-9]{24,}\b/i.test(basename)) return null;
  const cleaned = basename.replace(/[\u0000-\u001F\u007F{}<>`]/g, "").trim();
  if (!cleaned) return null;
  return cleaned.length > 80 ? `${cleaned.slice(0, 79).trimEnd()}…` : cleaned;
}

function safePromptText(value: unknown, maxLength = 140) {
  if (typeof value !== "string") return null;
  if (/\b(?:bearer|api[ _-]?key|password|secret|token)\b/i.test(value)) return null;
  if (/\b[a-f0-9]{24,}\b/i.test(value)) return null;
  if (
    /(?:&&|\|\||\$\(|(?:^|\s)--[a-z]|(?:^|\s)(?:sudo|curl|wget|rm|chmod|chown|bash|zsh|python|node|osascript|powershell)\s)/i.test(value)
  ) return null;
  const cleaned = value
    .replace(/(?:```|~~~)[\s\S]*?(?:```|~~~)/g, " ")
    .replace(/(?:```|~~~)[\s\S]*$/g, " ")
    .replace(/`[^`]*`/g, " ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])\s+/gm, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/\bhttps?:\/\/\S+/gi, " ")
    .replace(/[\u0000-\u001F\u007F{}]/g, " ")
    .replace(/[*_~]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return null;
  return cleaned.length > maxLength
    ? `${cleaned.slice(0, maxLength - 1).trimEnd()}…`
    : cleaned;
}

function sanitizeTechnicalText(value: string) {
  return value
    .replace(/\bwfi-[A-Za-z0-9:_-]+/g, "this run")
    .replace(/\bwf-[A-Za-z0-9:_-]+/g, "this workflow")
    .replace(/\bnodes\.[A-Za-z0-9:_-]+\.output\b/g, "the previous step's result")
    .replace(/\b[a-f0-9]{24,}\b/gi, "the saved item")
    .replace(/\s+/g, " ")
    .trim();
}
