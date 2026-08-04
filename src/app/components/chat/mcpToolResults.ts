import type { McpToolCallResult } from "@/hooks/useMcp";

const maximumResultCharacters = 24_000;

type ToolResultCopy = {
  toolFailureWithoutDetails: string;
  toolResultMissing: string;
};

type NormalizedMcpPayload = {
  content: string[];
  failures: unknown[];
};

export type NativeMcpExecutionReceipt = {
  receiptId: string;
  capabilityId: string;
  outcome: "succeeded" | "failed" | "unmet" | "unsupported";
  verified: boolean;
  nativeResultCode: string | null;
};

export type NativeMcpPermissionFailure = {
  capabilityId: string;
  code: string;
};

export type VerifiedSovereignMcpSearchResult = {
  query: string;
  engine: string;
  resultCount: number;
  contextJson: string;
  degraded: false;
  receiptDigest: string;
  invocationIndex: number;
};

type LocalToolFailureCode =
  | "timeout"
  | "permission"
  | "unavailable"
  | "failed";

class LocalToolResultError extends Error {
  readonly code: LocalToolFailureCode;

  constructor(code: LocalToolFailureCode, message: string) {
    super(message);
    this.name = "LocalToolResultError";
    this.code = code;
  }
}

function safeJsonStringify(value: unknown) {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return "";
  }
}

function containsMcpEnvelope(value: unknown, depth = 0): boolean {
  if (depth > 12 || value == null) return false;
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value);
      return parsed !== value && containsMcpEnvelope(parsed, depth + 1);
    } catch {
      return false;
    }
  }
  if (Array.isArray(value)) {
    return value.slice(0, 256).some((entry) => containsMcpEnvelope(entry, depth + 1));
  }
  if (typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    "isError" in record ||
    "is_error" in record ||
    "structuredContent" in record ||
    "structured_content" in record ||
    Object.values(record).some((entry) => containsMcpEnvelope(entry, depth + 1))
  );
}

function collectNormalizedMcpPayload(
  value: unknown,
  normalized: NormalizedMcpPayload,
  depth: number,
) {
  if (depth > 12 || value == null) return;
  if (typeof value === "string") {
    const text = value.trim();
    if (!text || text.toLowerCase() === "null") return;
    try {
      const parsed = JSON.parse(text);
      if (parsed !== value) {
        if (parsed == null) return;
        if (containsMcpEnvelope(parsed, depth + 1) || typeof parsed === "string") {
          collectNormalizedMcpPayload(parsed, normalized, depth + 1);
        } else {
          const serialized = safeJsonStringify(parsed).trim();
          if (serialized && serialized.toLowerCase() !== "null") {
            normalized.content.push(serialized);
          }
        }
        return;
      }
    } catch {
      // Plain text is already the safest representation.
    }
    normalized.content.push(text);
    return;
  }
  if (Array.isArray(value)) {
    value
      .slice(0, 256)
      .forEach((entry) => collectNormalizedMcpPayload(entry, normalized, depth + 1));
    return;
  }
  if (typeof value !== "object") {
    normalized.content.push(String(value));
    return;
  }

  const record = value as Record<string, unknown>;
  const isEnvelope =
    "isError" in record ||
    "is_error" in record ||
    "structuredContent" in record ||
    "structured_content" in record;
  if (isEnvelope) {
    if (record.isError === true || record.is_error === true) {
      normalized.failures.push(record);
      return;
    }
    const contentPayload: NormalizedMcpPayload = { content: [], failures: [] };
    const structuredPayload: NormalizedMcpPayload = { content: [], failures: [] };
    collectNormalizedMcpPayload(record.content, contentPayload, depth + 1);
    collectNormalizedMcpPayload(
      record.structuredContent ?? record.structured_content,
      structuredPayload,
      depth + 1,
    );
    normalized.failures.push(...contentPayload.failures, ...structuredPayload.failures);
    normalized.content.push(
      ...(structuredPayload.content.length > 0
        ? structuredPayload.content
        : contentPayload.content),
    );
    return;
  }

  if ("text" in record && (record.type === "text" || Object.keys(record).length <= 2)) {
    collectNormalizedMcpPayload(record.text, normalized, depth + 1);
    return;
  }
  const serialized = safeJsonStringify(record).trim();
  if (serialized && serialized.toLowerCase() !== "null") normalized.content.push(serialized);
}

function normalizeMcpPayload(value: unknown) {
  const normalized: NormalizedMcpPayload = { content: [], failures: [] };
  collectNormalizedMcpPayload(value, normalized, 0);
  return normalized;
}

function successfulStructuredContent(value: unknown, depth = 0): Record<string, unknown> | null {
  if (depth > 8 || value == null) return null;
  if (typeof value === "string") {
    try {
      return successfulStructuredContent(JSON.parse(value), depth + 1);
    } catch {
      return null;
    }
  }
  if (Array.isArray(value)) {
    for (const entry of value.slice(0, 64)) {
      const structured = successfulStructuredContent(entry, depth + 1);
      if (structured) return structured;
    }
    return null;
  }
  if (typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (record.isError === true || record.is_error === true) return null;
  const structured = record.structuredContent ?? record.structured_content;
  if (structured && typeof structured === "object" && !Array.isArray(structured)) {
    return structured as Record<string, unknown>;
  }
  return successfulStructuredContent(record.content, depth + 1);
}

function nonEmptyString(value: unknown) {
  return typeof value === "string" && value.trim().length > 0;
}

const emptySha256 =
  "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

function plainRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

export function nativeMcpExecutionReceipt(
  result: McpToolCallResult,
): NativeMcpExecutionReceipt | null {
  const metadata = plainRecord(result._meta);
  const receipt = plainRecord(metadata?.oomuNativeExecutionReceipt);
  if (receipt?.schema !== "oomu.native-mcp-execution.v1") return null;
  const receiptId = typeof receipt.receiptId === "string"
    ? receipt.receiptId.trim()
    : "";
  const capabilityId = typeof receipt.capabilityId === "string"
    ? receipt.capabilityId.trim()
    : "";
  const outcome = receipt.outcome;
  if (
    !receiptId || receiptId.length > 256 ||
    !/^[a-z][a-z0-9_]{0,79}$/.test(capabilityId) ||
    !["succeeded", "failed", "unmet", "unsupported"].includes(String(outcome))
  ) {
    return null;
  }
  const postcondition = plainRecord(receipt.postcondition);
  return {
    receiptId,
    capabilityId,
    outcome: outcome as NativeMcpExecutionReceipt["outcome"],
    verified: outcome === "succeeded" && receipt.verified === true,
    nativeResultCode: typeof postcondition?.nativeResultCode === "string"
      ? postcondition.nativeResultCode.trim() || null
      : null,
  };
}

export function nativeMcpPermissionFailure(
  receipt: NativeMcpExecutionReceipt | null,
): NativeMcpPermissionFailure | null {
  const code = receipt?.nativeResultCode?.trim().toLowerCase() ?? "";
  if (
    receipt?.outcome !== "unmet" ||
    !code ||
    !/(?:permission|authorization|access)/.test(code)
  ) {
    return null;
  }
  return { capabilityId: receipt.capabilityId, code };
}

function positiveSafeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) > 0;
}

function verifiedPublicSearchContext(value: string) {
  if (!value.trim() || value.length > 2_000_000) return false;
  try {
    const context = plainRecord(JSON.parse(value));
    if (!context || !nonEmptyString(context.accessedAtUtc)) return false;
    const pages = Array.isArray(context.pages) ? context.pages : [];
    return pages.some((page) => {
      const url = plainRecord(page)?.url;
      if (!nonEmptyString(url)) return false;
      try {
        return new URL(String(url)).protocol === "https:";
      } catch {
        return false;
      }
    });
  } catch {
    return false;
  }
}

/**
 * Accepts public-search context only when the native broker supplies a strict,
 * matching sovereign-search receipt. Tool names and result prose are never
 * sufficient to promote a generic MCP result to public provenance.
 */
export function verifiedSovereignMcpSearchResult(
  result: McpToolCallResult,
): VerifiedSovereignMcpSearchResult | null {
  if (result.isError) return null;
  const structured = plainRecord(result.structuredContent);
  const search = plainRecord(structured?.sovereignSearch);
  const metadata = plainRecord(result._meta);
  const receipt = plainRecord(metadata?.oomuSovereignSearchReceipt);
  if (
    !search || !receipt ||
    receipt.schema !== "oomu.sovereign-mcp-search.v1" ||
    receipt.verified !== true ||
    search.degraded !== false
  ) {
    return null;
  }

  const query = typeof search.query === "string" ? search.query.trim() : "";
  const engine = typeof search.engine === "string" ? search.engine.trim() : "";
  const contextJson = typeof search.contextJson === "string" ? search.contextJson : "";
  const receiptDigest = typeof search.receiptDigest === "string"
    ? search.receiptDigest
    : "";
  const receiptQuery = typeof receipt.query === "string" ? receipt.query.trim() : "";
  const receiptEngine = typeof receipt.engine === "string" ? receipt.engine.trim() : "";
  if (
    !query || query.length > 500 ||
    !engine || engine.length > 128 ||
    !positiveSafeInteger(search.resultCount) ||
    !positiveSafeInteger(search.invocationIndex) ||
    !/^[a-f0-9]{64}$/.test(receiptDigest) ||
    receiptQuery !== query ||
    receiptEngine !== engine ||
    receipt.resultCount !== search.resultCount ||
    receipt.receiptDigest !== receiptDigest ||
    receipt.invocationIndex !== search.invocationIndex ||
    !verifiedPublicSearchContext(contextJson)
  ) {
    return null;
  }

  return {
    query,
    engine,
    resultCount: search.resultCount,
    contextJson,
    degraded: false,
    receiptDigest,
    invocationIndex: search.invocationIndex,
  };
}

function normalizedRelativePath(value: unknown) {
  if (typeof value !== "string") return "";
  return value.trim().replace(/\\/g, "/").replace(/^\.\//, "");
}

function verifiedExplicitEmptyWrite(
  structured: Record<string, unknown>,
  approvedArguments: unknown,
) {
  const approved = plainRecord(approvedArguments);
  if (!approved || approved.content !== "") return false;
  const requestedPath = normalizedRelativePath(approved.path);
  const receiptPath = normalizedRelativePath(
    structured.relativePath ?? structured.path,
  );
  return Boolean(
    requestedPath &&
    receiptPath === requestedPath &&
    structured.bytesWritten === 0 &&
    structured.exists === true &&
    structured.verified === true &&
    structured.contentSha256 === emptySha256 &&
    structured.targetIdentityVerified === true,
  );
}

const mcpMutationToolKeys = new Set([
  "local_filesystem/write_file",
  "local_filesystem/delete_file",
  "macos_applescript/add_system_reminder",
  "macos_applescript/create_system_note",
  "macos_applescript/draft_system_email",
]);

export function conversationalMcpToolIsMutation(serverName: string, toolName: string) {
  return mcpMutationToolKeys.has(
    `${serverName.trim().toLowerCase()}/${toolName.trim().toLowerCase()}`,
  );
}

export function mcpMutationResultHasVerifiedPostcondition(
  serverName: string,
  toolName: string,
  result: McpToolCallResult,
  approvedArguments?: unknown,
) {
  if (normalizeMcpPayload(result).failures.length > 0) return false;
  const structured = successfulStructuredContent(result);
  if (!structured) return false;
  const key = `${serverName.trim().toLowerCase()}/${toolName.trim().toLowerCase()}`;
  switch (key) {
    case "local_filesystem/write_file":
      return nonEmptyString(structured.path ?? structured.relativePath) && (
        (typeof structured.bytesWritten === "number" && structured.bytesWritten > 0) ||
        verifiedExplicitEmptyWrite(structured, approvedArguments)
      );
    case "local_filesystem/delete_file":
      return nonEmptyString(structured.path ?? structured.relativePath) && structured.deleted === true;
    case "macos_applescript/add_system_reminder":
    case "macos_applescript/create_system_note":
      return nonEmptyString(structured.id) && nonEmptyString(structured.title);
    case "macos_applescript/draft_system_email":
      return structured.success === true && structured.saved === true &&
        structured.verified === true && nonEmptyString(structured.draftId) &&
        nonEmptyString(structured.subject);
    default:
      return false;
  }
}

function failureCodeFromText(value: string, depth = 0): LocalToolFailureCode | null {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return null;
  try {
    const parsed = JSON.parse(value);
    if (parsed !== value) return localToolFailureCodeFromUnknown(parsed, depth + 1);
  } catch {
    // Plain messages are classified without displaying the payload.
  }
  const words = normalized.replace(/[_-]+/g, " ");
  if (/\b(?:timed?\s*out|timeout)\b/.test(words)) return "timeout";
  if (
    /\b(?:permission|not authorized|authorization|access denied|denied|restricted)\b/.test(
      words,
    ) || /\baccess is (?:off|disabled)\b/.test(words)
  ) {
    return "permission";
  }
  if (
    /\b(?:unavailable|disconnected|not found|could not connect|desktop required)\b/.test(
      words,
    ) || /\bavailable only (?:in|on)\b/.test(words)
  ) {
    return "unavailable";
  }
  return null;
}

function localToolFailureCodeFromUnknown(
  value: unknown,
  depth = 0,
): LocalToolFailureCode | null {
  if (depth > 12 || value == null) return null;
  if (value instanceof LocalToolResultError) return value.code;
  if (typeof value === "string") return failureCodeFromText(value, depth);
  if (Array.isArray(value)) {
    for (const item of value.slice(0, 24)) {
      const code = localToolFailureCodeFromUnknown(item, depth + 1);
      if (code) return code;
    }
    return null;
  }
  if (typeof value !== "object") return null;

  const record = value as Record<string, unknown>;
  for (const key of [
    "code",
    "errorCode",
    "error_code",
    "warning",
    "errorType",
    "error_type",
    "status",
  ]) {
    const code = localToolFailureCodeFromUnknown(record[key], depth + 1);
    if (code) return code;
  }
  for (const key of [
    "message",
    "error",
    "text",
    "content",
    "structuredContent",
    "structured_content",
  ]) {
    const code = localToolFailureCodeFromUnknown(record[key], depth + 1);
    if (code) return code;
  }
  return null;
}

export function localToolFailureCode(value: unknown): LocalToolFailureCode {
  return localToolFailureCodeFromUnknown(value) ?? "failed";
}

export type ProtectedAppleLibraryToolName =
  | "read_system_contacts"
  | "read_system_music"
  | "read_system_photos";

function protectedAppleLibraryPrefix(toolName: ProtectedAppleLibraryToolName) {
  return toolName.replace("read_system_", "");
}

export function protectedAppleLibraryFailureKey(
  toolName: ProtectedAppleLibraryToolName,
  error: unknown,
) {
  const code = localToolFailureCode(error);
  const suffix =
    code === "permission"
      ? "permission"
      : code === "timeout"
        ? "timeout"
        : "unavailable";
  return `chat.errors.${protectedAppleLibraryPrefix(toolName)}_${suffix}`;
}

export function protectedAppleLibraryDesktopKey(
  toolName: ProtectedAppleLibraryToolName,
) {
  return `chat.errors.${protectedAppleLibraryPrefix(toolName)}_desktop_required`;
}

export function mcpToolResultText(result: McpToolCallResult, copy: ToolResultCopy) {
  const normalized = normalizeMcpPayload(result);
  if (normalized.failures.length > 0) {
    throw new LocalToolResultError(
      localToolFailureCode(normalized.failures),
      copy.toolFailureWithoutDetails,
    );
  }
  const content = [...new Set(normalized.content)].join("\n\n");
  if (!content) throw new Error(copy.toolResultMissing);
  if (content.length <= maximumResultCharacters) return content;
  return `${content.slice(0, maximumResultCharacters)}\n\n[Tool result truncated at ${maximumResultCharacters} characters.]`;
}
