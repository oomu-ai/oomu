type SearchTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

const searchErrorCodes = [
  "search_not_authorized",
  "search_query_invalid",
  "search_provider_challenge",
  "search_provider_unavailable",
  "search_retrieval_timeout",
  "search_no_results",
  "search_dom_failed",
  "search_cancelled",
  "search_unavailable",
] as const;

export type LocalSearchFailureCode = (typeof searchErrorCodes)[number];
export type LocalSearchFailureKind =
  | "blocked"
  | "no_results"
  | "unavailable"
  | "timed_out"
  | "cancelled";

export function localSearchFailureKind(
  errorCode: string | undefined,
): LocalSearchFailureKind {
  if (errorCode === "search_not_authorized" || errorCode === "search_query_invalid") {
    return "blocked";
  }
  if (errorCode === "search_no_results") {
    return "no_results";
  }
  if (errorCode === "search_retrieval_timeout") {
    return "timed_out";
  }
  if (errorCode === "search_cancelled") {
    return "cancelled";
  }
  return "unavailable";
}

export function localSearchFailureMessage(
  errorCode: string | undefined,
  t: SearchTranslate,
) {
  if (errorCode === "search_not_authorized") {
    return t("chat.search_errors.not_authorized");
  }
  if (errorCode === "search_query_invalid") {
    return t("chat.search_errors.query_invalid");
  }
  if (errorCode === "search_no_results") {
    return t("chat.search_errors.no_results");
  }
  if (errorCode === "search_retrieval_timeout") {
    return t("chat.search_errors.timed_out");
  }
  if (errorCode === "search_cancelled") {
    return t("chat.search_errors.cancelled");
  }
  if (
    errorCode === "search_provider_challenge" ||
    errorCode === "search_provider_unavailable"
  ) {
    return t("chat.search_errors.provider_unavailable");
  }
  if (errorCode === "search_dom_failed") {
    return t("chat.search_errors.source_unreadable");
  }
  return t("chat.search_errors.unavailable");
}

export function localSearchTerminalStatus(
  errorCode: string | undefined,
): "completed" | "failed" | "cancelled" {
  if (errorCode === "search_cancelled") return "cancelled";
  return ["search_no_results", "search_query_invalid"].includes(errorCode ?? "")
    ? "completed"
    : "failed";
}

export function localSearchFailureCode(error: unknown): LocalSearchFailureCode {
  const code = searchFailureCodeFromUnknown(error);
  return searchErrorCodes.includes(code as LocalSearchFailureCode)
    ? code as LocalSearchFailureCode
    : "search_unavailable";
}

function searchFailureCodeFromUnknown(error: unknown, depth = 0): string {
  if (depth > 4 || error === null || error === undefined) return "";
  if (typeof error === "string") {
    const normalized = error.trim();
    if (!normalized || normalized.length > 16_000) return "";
    if (searchErrorCodes.includes(normalized as LocalSearchFailureCode)) return normalized;
    if (!normalized.startsWith("{") && !normalized.startsWith("[")) return "";
    try {
      return searchFailureCodeFromUnknown(JSON.parse(normalized), depth + 1);
    } catch {
      return "";
    }
  }
  if (typeof error !== "object") return "";
  const record = error as Record<string, unknown>;
  for (const key of ["code", "errorCode", "error_code"]) {
    const candidate = record[key];
    if (
      typeof candidate === "string" &&
      searchErrorCodes.includes(candidate as LocalSearchFailureCode)
    ) {
      return candidate;
    }
  }
  for (const key of ["error", "cause", "details", "data"]) {
    const nested = searchFailureCodeFromUnknown(record[key], depth + 1);
    if (nested) return nested;
  }
  return "";
}
