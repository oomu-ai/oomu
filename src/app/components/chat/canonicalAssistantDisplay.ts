const FENCED_BLOCK = /(```[^\n]*\n[\s\S]*?```|~~~[^\n]*\n[\s\S]*?~~~)/g;

const INTERNAL_ENVELOPE_NAMES = [
  "tool_call",
  "tool_result",
  "native_receipt",
  "execution_receipt",
  "internal_directive",
  "oomu_control",
  "mcp_call",
  "function_call",
] as const;

function internalEnvelopePattern(name: string) {
  return new RegExp(`<\\s*${name}\\b[^>]*>[\\s\\S]*?<\\s*\\/\\s*${name}\\s*>`, "gi");
}

function orphanInternalTagPattern(name: string) {
  return new RegExp(`<\\s*\\/?\\s*${name}\\b[^>]*>`, "gi");
}

function unclosedInternalEnvelopePattern(name: string) {
  return new RegExp(`<\\s*${name}\\b[^>]*>[\\s\\S]*$`, "gi");
}

function stripInternalEnvelopes(text: string) {
  let cleaned = text;
  for (const name of INTERNAL_ENVELOPE_NAMES) {
    cleaned = cleaned.replace(internalEnvelopePattern(name), "");
    // During streaming, an internal envelope can arrive before its closing
    // tag. Once a recognized start tag appears, nothing after it is safe to
    // present until the envelope is complete.
    cleaned = cleaned.replace(unclosedInternalEnvelopePattern(name), "");
    cleaned = cleaned.replace(orphanInternalTagPattern(name), "");
  }
  return cleaned;
}

function repairUnambiguousMarkdownBoundaries(text: string) {
  return text
    // Some providers occasionally fuse the English word "named" to an OOMU
    // artifact filename in their final (non-streamed) response. This is a
    // narrow, user-visible boundary repair; ordinary identifiers and fenced
    // examples remain byte-for-byte unchanged.
    .replace(/\bnamed(?=oomu[_-])/gi, "$& ")
    // Repair a heading only when a complete Markdown heading marker was
    // visibly concatenated to prose. URLs and ordinary # characters do not
    // match because Markdown requires whitespace after the marker.
    .replace(/([^#\n])(?=#{1,6}[ \t]+\S)/g, "$1\n\n")
    // A divider must occupy its own line. Keep inline hyphens untouched.
    .replace(/([^\n])(?=---[ \t]*(?:\n|$))/g, "$1\n\n")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n");
}

/**
 * Produces the one visible form of assistant text. Fenced examples are
 * treated as literal user content, so protocol-looking samples remain exact.
 * Only recognized internal envelopes and unambiguous Markdown boundary
 * defects are changed.
 */
export function canonicalAssistantDisplayText(content: string) {
  return content
    .split(FENCED_BLOCK)
    .map((part, index) => {
      const isFence = index % 2 === 1;
      return isFence
        ? part
        : repairUnambiguousMarkdownBoundaries(stripInternalEnvelopes(part));
    })
    .join("")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
