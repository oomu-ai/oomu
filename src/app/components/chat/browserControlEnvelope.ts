export type BrowserControlEnvelopeOutcome =
  | "clear"
  | "buffering"
  | "complete"
  | "malformed";

export type BrowserControlEnvelopeProjection = {
  visibleText: string;
  directiveSnapshot: string;
  outcome: BrowserControlEnvelopeOutcome;
};

export type BrowserControlEnvelopeUpdate = BrowserControlEnvelopeProjection & {
  visibleDelta: string;
  directiveChanged: boolean;
};

const RAW_TAG = /<\s*(\/?)\s*oomusplitview\b[^>]*>/gi;
const ENCODED_TAG = /&(?:amp;)?lt;\s*(\/?)\s*oomusplitview\b[\s\S]*?&(?:amp;)?gt;/gi;
const RAW_OPENING_PREFIX = "<oomusplitview";
const RAW_CLOSING_PREFIX = "</oomusplitview";
const ENCODED_OPENING_PREFIX = "&lt;oomusplitview";
const ENCODED_CLOSING_PREFIX = "&lt;/oomusplitview";
const DOUBLE_ENCODED_OPENING_PREFIX = "&amp;lt;oomusplitview";
const DOUBLE_ENCODED_CLOSING_PREFIX = "&amp;lt;/oomusplitview";
const TYPED_CONTROL_FENCE_NAMES = [
  "oomu_search_request",
  "oomu_mcp_tool_call",
] as const;
const TYPED_CONTROL_FENCE_HEADER =
  /^[ \t]*(?:json[ \t]+)?(?:oomu_search_request|oomu_mcp_tool_call)[^\n]*$/i;

type ProtocolTag = {
  start: number;
  end: number;
  closing: boolean;
  encoded: boolean;
};

function nextProtocolTag(content: string, startIndex: number): ProtocolTag | null {
  RAW_TAG.lastIndex = startIndex;
  ENCODED_TAG.lastIndex = startIndex;
  const raw = RAW_TAG.exec(content);
  const encoded = ENCODED_TAG.exec(content);
  const match = !raw
    ? encoded
    : !encoded || raw.index <= encoded.index
      ? raw
      : encoded;
  if (!match) return null;
  return {
    start: match.index,
    end: match.index + match[0].length,
    closing: match[1] === "/",
    encoded: match === encoded,
  };
}

function matchingClosingTag(
  content: string,
  opening: ProtocolTag,
): ProtocolTag | null {
  let cursor = opening.end;
  while (cursor < content.length) {
    const candidate = nextProtocolTag(content, cursor);
    if (!candidate) return null;
    if (candidate.encoded === opening.encoded && candidate.closing) {
      return candidate;
    }
    cursor = candidate.end;
  }
  return null;
}

function normalizedPotentialMarker(value: string) {
  return value.toLowerCase().replace(/\s+/g, "");
}

function trailingPotentialMarkerIndex(content: string, startIndex: number) {
  const prefixes = [
    RAW_OPENING_PREFIX,
    RAW_CLOSING_PREFIX,
    ENCODED_OPENING_PREFIX,
    ENCODED_CLOSING_PREFIX,
    DOUBLE_ENCODED_OPENING_PREFIX,
    DOUBLE_ENCODED_CLOSING_PREFIX,
  ];
  const earliestCandidate = Math.max(startIndex, content.length - 32);
  for (let index = earliestCandidate; index < content.length; index += 1) {
    if (content[index] !== "<" && content[index] !== "&") {
      continue;
    }
    const suffix = normalizedPotentialMarker(content.slice(index));
    if (prefixes.some((prefix) => prefix.startsWith(suffix))) {
      return index;
    }
  }
  return -1;
}

function isPotentialTypedControlFenceHeader(header: string) {
  const value = header.replace(/^[ \t]+/, "").toLowerCase();
  if (!value) return true;
  if (TYPED_CONTROL_FENCE_NAMES.some(
    (name) => name.startsWith(value) || value.startsWith(name),
  )) {
    return true;
  }
  if ("json".startsWith(value)) return true;
  if (!value.startsWith("json")) return false;
  const separator = value.slice(4).match(/^[ \t]+/);
  if (!separator) return value === "json";
  const typedHeader = value.slice(4 + separator[0].length);
  return !typedHeader || TYPED_CONTROL_FENCE_NAMES.some(
    (name) => name.startsWith(typedHeader) || typedHeader.startsWith(name),
  );
}

function hasTypedControlFencePrefix(header: string) {
  let value = header.replace(/^[ \t]+/, "").toLowerCase();
  if (value.startsWith("json")) {
    const separator = value.slice(4).match(/^[ \t]+/);
    if (!separator) return false;
    value = value.slice(4 + separator[0].length);
  }
  return Boolean(value) && TYPED_CONTROL_FENCE_NAMES.some(
    (name) => name.startsWith(value) || value.startsWith(name),
  );
}

function trailingFencePrefixIndex(content: string, startIndex: number) {
  if (content.length > startIndex && content.endsWith("``")) {
    return content.length - 2;
  }
  if (content.length > startIndex && content.endsWith("`")) {
    return content.length - 1;
  }
  return -1;
}

function projectTypedControlFences(content: string, final: boolean) {
  let visibleText = "";
  let cursor = 0;
  let sawComplete = false;

  while (cursor < content.length) {
    const opening = content.indexOf("```", cursor);
    if (opening < 0) {
      const partialIndex = trailingFencePrefixIndex(content, cursor);
      if (partialIndex >= 0) {
        if (final) {
          visibleText += content.slice(cursor);
          break;
        }
        visibleText += content.slice(cursor, partialIndex);
        return {
          visibleText,
          outcome: "buffering",
        } as const;
      }
      visibleText += content.slice(cursor);
      break;
    }

    visibleText += content.slice(cursor, opening);
    const headerStart = opening + 3;
    const headerEnd = content.indexOf("\n", headerStart);
    if (headerEnd < 0) {
      const header = content.slice(headerStart);
      if (isPotentialTypedControlFenceHeader(header)) {
        if (final && !hasTypedControlFencePrefix(header)) {
          visibleText += "```";
          cursor = headerStart;
          continue;
        }
        return {
          visibleText,
          outcome: final ? "malformed" : "buffering",
        } as const;
      }
      visibleText += "```";
      cursor = headerStart;
      continue;
    }

    const header = content.slice(headerStart, headerEnd);
    if (!TYPED_CONTROL_FENCE_HEADER.test(header)) {
      visibleText += "```";
      cursor = headerStart;
      continue;
    }

    const closing = content.indexOf("```", headerEnd + 1);
    if (closing < 0) {
      return {
        visibleText,
        outcome: final ? "malformed" : "buffering",
      } as const;
    }
    sawComplete = true;
    cursor = closing + 3;
  }

  return {
    visibleText,
    outcome: sawComplete ? "complete" : "clear",
  } as const;
}

function combineEnvelopeOutcomes(
  first: BrowserControlEnvelopeOutcome,
  second: BrowserControlEnvelopeOutcome,
) {
  if (first === "malformed" || second === "malformed") return "malformed";
  if (first === "buffering" || second === "buffering") return "buffering";
  if (first === "complete" || second === "complete") return "complete";
  return "clear";
}

function projectSplitViewEnvelope(
  content: string,
  final = false,
): BrowserControlEnvelopeProjection {
  let visibleText = "";
  let directiveSnapshot = "";
  let cursor = 0;
  let sawComplete = false;
  let sawMalformed = false;

  while (cursor < content.length) {
    const marker = nextProtocolTag(content, cursor);
    if (!marker) {
      const partialIndex = trailingPotentialMarkerIndex(content, cursor);
      if (partialIndex >= 0) {
        visibleText += content.slice(cursor, partialIndex);
        if (final) sawMalformed = true;
        return {
          visibleText,
          directiveSnapshot,
          outcome: final ? "malformed" : "buffering",
        };
      }
      visibleText += content.slice(cursor);
      break;
    }

    visibleText += content.slice(cursor, marker.start);
    if (marker.closing) {
      sawMalformed = true;
      cursor = marker.end;
      continue;
    }

    const closing = matchingClosingTag(content, marker);
    if (!closing) {
      if (final) sawMalformed = true;
      return {
        visibleText,
        directiveSnapshot,
        outcome: final ? "malformed" : "buffering",
      };
    }

    const block = content.slice(marker.start, closing.end);
    if (marker.encoded) {
      sawMalformed = true;
    } else {
      directiveSnapshot += block;
      sawComplete = true;
    }
    cursor = closing.end;
  }

  return {
    visibleText,
    directiveSnapshot,
    outcome: sawMalformed ? "malformed" : sawComplete ? "complete" : "clear",
  };
}

export function projectBrowserControlEnvelope(
  content: string,
  final = false,
): BrowserControlEnvelopeProjection {
  const typedControl = projectTypedControlFences(content, final);
  const splitView = projectSplitViewEnvelope(typedControl.visibleText, final);
  return {
    ...splitView,
    outcome: combineEnvelopeOutcomes(typedControl.outcome, splitView.outcome),
  };
}

export class BrowserControlEnvelopeAccumulator {
  private content = "";
  private visibleText = "";
  private directiveSnapshot = "";

  push(chunk: string): BrowserControlEnvelopeUpdate {
    return this.project(chunk, false);
  }

  finish(): BrowserControlEnvelopeUpdate {
    return this.project("", true);
  }

  private project(chunk: string, final: boolean): BrowserControlEnvelopeUpdate {
    this.content += chunk;
    const projection = projectBrowserControlEnvelope(this.content, final);
    const visibleDelta = projection.visibleText.startsWith(this.visibleText)
      ? projection.visibleText.slice(this.visibleText.length)
      : "";
    const directiveChanged = projection.directiveSnapshot !== this.directiveSnapshot;
    this.visibleText = projection.visibleText;
    this.directiveSnapshot = projection.directiveSnapshot;
    return { ...projection, visibleDelta, directiveChanged };
  }
}
