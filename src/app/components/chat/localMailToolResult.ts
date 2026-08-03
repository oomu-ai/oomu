import type { McpToolCallResult } from "@/hooks/useMcp";
import {
  localToolFailureCode,
  mcpToolResultText,
} from "./mcpToolResults";

type ToolResultCopy = {
  toolFailureWithoutDetails: string;
  toolResultMissing: string;
};

function safeJsonStringify(value: unknown) {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return "";
  }
}

function hasMailFailureDetail(value: unknown, depth = 0): boolean {
  if (depth > 6 || value == null) return false;
  if (typeof value === "string") {
    try {
      return hasMailFailureDetail(JSON.parse(value), depth + 1);
    } catch {
      return value.trim().length > 0;
    }
  }
  if (Array.isArray(value)) {
    return value.some((entry) => hasMailFailureDetail(entry, depth + 1));
  }
  if (typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  if (["error", "message", "warning", "detail"].some((key) => record[key] != null)) {
    return true;
  }
  return Object.values(record).some((entry) => hasMailFailureDetail(entry, depth + 1));
}

function mailContentText(content: unknown[]) {
  return content
    .flatMap((entry) => {
      if (typeof entry === "string") return [entry.trim()];
      if (!entry || typeof entry !== "object") return [];
      const text = (entry as Record<string, unknown>).text;
      return typeof text === "string" ? [text.trim()] : [];
    })
    .filter(Boolean)
    .join("\n\n");
}

/**
 * Permission and timeout envelopes still throw so the durable macOS recovery
 * card owns those states. Other native Mail failures keep their bounded detail
 * so the chat cannot misreport a failed read as an empty inbox.
 */
export function localMailToolResultText(
  result: McpToolCallResult,
  copy: ToolResultCopy,
) {
  if (!result.isError) {
    return mcpToolResultText(result, copy);
  }

  const code = localToolFailureCode(result);
  if (code === "permission" || code === "timeout") {
    return mcpToolResultText(result, copy);
  }

  if (hasMailFailureDetail(result.structuredContent)) {
    const structured = safeJsonStringify(result.structuredContent).trim();
    if (structured) return structured;
  }
  const content = mailContentText(result.content);
  if (content) return content;
  return mcpToolResultText(result, copy);
}

export function localMailFailureKey(error: unknown) {
  switch (localToolFailureCode(error)) {
    case "timeout":
      return "chat.errors.mail_timeout";
    case "permission":
      return "chat.errors.mail_permission";
    case "unavailable":
      return "chat.errors.mail_unavailable";
    default:
      return "chat.errors.mail_failed";
  }
}
