const REDACTED = "[redacted]";
const MAX_REDACTION_DEPTH = 12;
const MAX_REDACTION_NODES = 2048;
const MAX_REDACTION_INPUT_CHARS = 64 * 1024;
const MAX_REDACTION_OUTPUT_CHARS = 4096;
const TRUNCATED = "...[truncated]";
const STRUCTURE_LIMIT = "[redacted-structure-limit]";
const SENSITIVE_KEY_PARTS = [
  "authorization",
  "privatekey",
  "apikey",
  "password",
  "passwd",
  "credential",
  "secret",
  "token",
  "cookie",
] as const;

function normalizedKey(key: string) {
  return key.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function truncateText(value: string, maxChars = MAX_REDACTION_OUTPUT_CHARS) {
  if (value.length <= maxChars) return value;
  let end = Math.max(0, maxChars - TRUNCATED.length);
  const finalCodeUnit = value.charCodeAt(end - 1);
  if (finalCodeUnit >= 0xd800 && finalCodeUnit <= 0xdbff) end -= 1;
  return `${value.slice(0, end)}${TRUNCATED}`;
}

function isSensitiveKey(key: string) {
  const normalized = normalizedKey(key);
  return SENSITIVE_KEY_PARTS.some((part) => normalized.includes(part));
}

function redactUrl(rawUrl: string) {
  const trailing = rawUrl.match(/[)\]},.;]+$/)?.[0] ?? "";
  const candidate = trailing ? rawUrl.slice(0, -trailing.length) : rawUrl;
  try {
    const url = new URL(candidate);
    if (url.username) url.username = REDACTED;
    if (url.password) url.password = REDACTED;
    url.pathname = url.pathname.replace(
      /\/bot[0-9]{5,}:[A-Za-z0-9_-]{12,}/gi,
      `/bot${REDACTED}`,
    );
    for (const key of [...url.searchParams.keys()]) {
      if (normalizedKey(key) === "key" || isSensitiveKey(key)) {
        url.searchParams.set(key, REDACTED);
      }
    }
    return `${url.toString().replaceAll("%5Bredacted%5D", REDACTED)}${trailing}`;
  } catch {
    return rawUrl.replace(
      /(\/bot)[0-9]{5,}:[A-Za-z0-9_-]{12,}/gi,
      `$1${REDACTED}`,
    );
  }
}

export function redactSensitiveText(value: string): string {
  const boundedInput = truncateText(value, MAX_REDACTION_INPUT_CHARS);
  try {
    const parsed = JSON.parse(boundedInput) as unknown;
    if (parsed && typeof parsed === "object") {
      return truncateText(JSON.stringify(redactSensitiveValue(parsed)));
    }
  } catch {
    // Free text follows the reviewed URL/header/assignment contract below.
  }
  const urlsRedacted = boundedInput.replace(/https?:\/\/[^\s<>"']+/gi, redactUrl);
  return truncateText(urlsRedacted
    .replace(
      /\b(authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-goog-api-key)\s*:[^\r\n]*/gi,
      `$1: ${REDACTED}`,
    )
    .replace(/\b(bearer|basic)\s+[A-Za-z0-9._~+/=:-]+/gi, `$1 ${REDACTED}`)
    .replace(
      /(--?(?:authorization|api[-_]?key|password|passwd|client[-_]?secret|secret|credentials?|access[-_]?token|refresh[-_]?token|token|cookie|private[-_]?key))(\s+|=)(?:"[^"]*"|'[^']*'|[^\s,;&}\]]+)/gi,
      `$1$2${REDACTED}`,
    )
    .replace(
      /(authorization|proxy[-_ ]?authorization|api[-_ ]?key|apikey|password|passwd|client[-_ ]?secret|secret|credentials?|access[-_ ]?token|refresh[-_ ]?token|token|set[-_ ]?cookie|cookie|private[-_ ]?key)\s*([:=])\s*(?:"[^"]*"|'[^']*'|[^\s,;&}\]]+)/gi,
      `$1$2${REDACTED}`,
    )
    .replace(/(\/bot)[0-9]{5,}:[A-Za-z0-9_-]{12,}/gi, `$1${REDACTED}`)
    .replace(/\/(?:Users|home)\/[^/\s:]+/g, "/[home]")
    .replace(/[A-Z]:\\Users\\[^\\\s:]+/gi, "[home]"));
}

function errorRecord(error: Error): Record<string, unknown> {
  const record: Record<string, unknown> = {
    name: error.name,
    message: error.message,
  };
  if (error.stack) record.stack = error.stack;
  if ("cause" in error) record.cause = error.cause;
  for (const key in error) {
    if (!Object.prototype.hasOwnProperty.call(error, key)) continue;
    try {
      record[key] = (error as unknown as Record<string, unknown>)[key];
    } catch {
      record[key] = "[unavailable]";
    }
  }
  return record;
}

type RedactionState = {
  seen: WeakSet<object>;
  nodes: number;
};

function redactUnknownInternal(
  value: unknown,
  depth: number,
  state: RedactionState,
): unknown {
  state.nodes += 1;
  if (state.nodes > MAX_REDACTION_NODES) return STRUCTURE_LIMIT;
  if (typeof value === "string") return redactSensitiveText(value);
  if (
    value === null ||
    value === undefined ||
    typeof value === "number" ||
    typeof value === "boolean" ||
    typeof value === "bigint"
  ) {
    return value;
  }
  if (typeof value === "function" || typeof value === "symbol") {
    return truncateText(String(value));
  }
  if (depth >= MAX_REDACTION_DEPTH) return STRUCTURE_LIMIT;
  if (typeof value !== "object") return redactSensitiveText(String(value));
  if (state.seen.has(value)) return "[circular]";
  state.seen.add(value);

  if (Array.isArray(value)) {
    const result: unknown[] = [];
    for (const entry of value) {
      if (state.nodes >= MAX_REDACTION_NODES) {
        result.push(STRUCTURE_LIMIT);
        break;
      }
      result.push(redactUnknownInternal(entry, depth + 1, state));
    }
    return result;
  }

  const source = value instanceof Error ? errorRecord(value) : value as Record<string, unknown>;
  const result: Record<string, unknown> = {};
  for (const key in source) {
    if (!Object.prototype.hasOwnProperty.call(source, key)) continue;
    if (state.nodes >= MAX_REDACTION_NODES) {
      result.__redaction_limit__ = STRUCTURE_LIMIT;
      break;
    }
    let entry: unknown;
    try {
      entry = source[key];
    } catch {
      entry = "[unavailable]";
    }
    result[key] = isSensitiveKey(key)
      ? REDACTED
      : redactUnknownInternal(entry, depth + 1, state);
  }
  return result;
}

export function redactSensitiveValue(value: unknown): unknown {
  return redactUnknownInternal(value, 0, { seen: new WeakSet(), nodes: 0 });
}

export function safeErrorMessage(error: unknown, fallback = "Operation failed.") {
  const redacted = redactSensitiveValue(error);
  if (typeof redacted === "string" && redacted.trim()) return redacted.trim();
  if (redacted && typeof redacted === "object") {
    const record = redacted as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim()) {
      return record.message.trim();
    }
    try {
      const serialized = JSON.stringify(record);
      if (serialized && serialized !== "{}") return truncateText(serialized);
    } catch {
      // Fall through to a constant message; raw error objects never cross the boundary.
    }
  }
  return fallback;
}
