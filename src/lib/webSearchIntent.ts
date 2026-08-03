import {
  hasLocalizedFreshnessIntent,
  localizedExplicitSearchQuery,
} from "./searchAuthorization/localeDirectives";

const externalSearchSurfaces = new Set([
  "web",
  "internet",
  "online",
  "google",
  "duckduckgo",
]);
const directPublicSearchActions = new Set([
  "browse",
  "check",
  "confirm",
  "find",
  "look",
  "research",
  "search",
  "see",
  "verify",
]);
const informationalOpeners = new Set([
  "how",
  "what",
  "why",
  "when",
  "where",
  "who",
  "did",
  "does",
  "do",
  "explain",
  "tell",
]);
const freshnessIntentPattern = /\b(latest|most recent|breaking|today|tonight|tomorrow|yesterday|this week|this month|this year|newest|recently|right now|at the moment|up to date|live|ongoing|currently|happening now|current events|current (?:price|prices|version|status|news|weather|score|scores|schedule|law|rule|regulation|ceo|president)|as of (?:today|20\d{2})|score|scores|standings|schedule|fixtures|market price|stock price|weather)\b/i;
const boundedPublicResearchPattern =
  /^research\s+(?:(?:current|recent|latest|public|primary|official|authoritative|web|or|and)\s+)+sources?\s+(?:(?:on|about|regarding|for)\s+|relevant\s+to\s+)(.+)$/i;
const externalResearchOptOutPattern =
  /\b(?:do not|don't|never)\s+(?:use|search|browse|research|access)\b[^.!?;\n]*\b(?:web|internet|online|sources?)\b|\bwithout\s+(?:using\s+)?(?:the\s+)?(?:web|internet|online)\b/i;
const retrospectiveSearchMentionPattern =
  /\b(?:i|we|you|oomu)\s+(?:searched|browsed)\s+(?:the\s+)?(?:web|internet|online|google|duckduckgo)\b/i;
const privateLocalDataTargetPattern = new RegExp(
  [
    String.raw`\b(?:my|our)\s+(?:apple\s+)?(?:calendars?|agenda|schedule|appointments?|events?|meetings?|inbox|mail|emails?|contacts?|address\s+book|photos?|photo\s+albums?|images?|pictures?|camera\s+roll|files?|folders?|documents?|notes?|reminders?|tasks?|todos?|to-dos?|messages?|imessages?|songs?|tracks?|albums?|music|music\s+library)\b`,
    String.raw`\b(?:calendar|mail|contacts?|photos?|notes?|reminders?|messages?)\s+app\b`,
    String.raw`\b(?:apple|macos|local)\s+(?:calendar|mail|contacts?|photos?|notes?|reminders?|messages?)\b`,
    String.raw`\b(?:google|outlook|icloud)\s+(?:calendars?|photos?|mail|inbox|contacts?|drive)\b`,
    String.raw`\b(?:gmail|outlook)\s+(?:inbox|mail|emails?)\b`,
  ].join("|"),
  "i",
);
const functionWords = new Set([
  "a",
  "about",
  "an",
  "and",
  "answer",
  "are",
  "can",
  "cela",
  "check",
  "confirm",
  "danach",
  "did",
  "do",
  "does",
  "eso",
  "for",
  "für",
  "have",
  "i",
  "in",
  "is",
  "it",
  "isso",
  "itu",
  "look",
  "me",
  "of",
  "on",
  "online",
  "please",
  "por",
  "search",
  "それ",
  "でそれ",
  "that",
  "the",
  "these",
  "this",
  "those",
  "to",
  "điều",
  "đó",
  "up",
  "verify",
  "was",
  "web",
  "were",
  "what",
  "where",
  "who",
  "you",
  "это",
  "це",
  "那个",
  "那個",
]);

type SearchContextMessage = {
  role?: string;
  content: string;
};

export type SearchSource =
  | { kind: "user_text" }
  | { kind: "public_web_result" }
  | { kind: "private_local"; source: string; digest: string }
  | { kind: "unknown_derived" };

type SearchAuthorizationInput = {
  utterance: string;
  searchControlEnabled: boolean;
  sources: SearchSource[];
};

export type SearchAuthorizationDecision = {
  allowed: boolean;
  reason:
    | "explicit_public_search"
    | "ambient_freshness_search"
    | "search_disabled"
    | "not_explicit"
    | "private_source"
    | "unknown_derived_source"
    | "weak_query";
  query?: string;
};

export function hasLocalWebSearchIntent(content: string) {
  const normalized = content.trim();
  if (!normalized) {
    return false;
  }
  return hasExplicitLocalWebSearchIntent(normalized);
}

export function hasExplicitLocalWebSearchIntent(content: string) {
  const normalized = content.trim();
  if (!normalized) {
    return false;
  }
  if (localizedExplicitSearchQuery(normalized)) {
    return true;
  }
  if (boundedPublicResearchQueryFromText(normalized)) {
    return true;
  }
  return normalized
    .split(/[.!?;\n]+/)
    .map(searchDirectiveTokens)
    .some(hasExplicitSearchDirective);
}

function searchDirectiveTokens(clause: string) {
  const tokens = clause
    .toLocaleLowerCase("en-US")
    .replace(/[’]/g, "'")
    .split(/[^a-z0-9']+/)
    .filter(Boolean);

  while (tokens.length > 0) {
    let prefixLength = 0;
    if (["please", "oomu"].includes(tokens[0])) {
      prefixLength = 1;
    } else if (
      ["can", "could", "would", "will"].includes(tokens[0]) &&
      tokens[1] === "you"
    ) {
      prefixLength = 2;
    } else if (
      tokens[0] === "i" &&
      ["want", "need"].includes(tokens[1]) &&
      tokens[2] === "you"
    ) {
      prefixLength = 3;
    } else if (
      tokens[0] === "go" &&
      tokens[1] === "ahead" &&
      tokens[2] === "and"
    ) {
      prefixLength = 3;
    }
    if (prefixLength === 0) {
      break;
    }
    tokens.splice(0, prefixLength);
    if (tokens[0] === "to") {
      tokens.shift();
    }
  }
  return tokens;
}

function hasExplicitSearchDirective(tokens: string[]) {
  const first = tokens[0];
  if (!first || informationalOpeners.has(first)) {
    return false;
  }

  if (directPublicSearchActions.has(first)) {
    if (
      first === "check" &&
      tokens[1] === "google" &&
      tokens[2] === "calendar"
    ) {
      return false;
    }
    if (first === "look") {
      const upIndex = tokens.indexOf("up");
      if (upIndex > 0 && hasLookUpExternalLocator(tokens, upIndex + 1)) {
        return true;
      }
    }
    return externalSurfaceAfter(tokens, 1) >= 0;
  }
  if (
    first === "go" &&
    tokens[1] === "online" &&
    tokens[2] === "and" &&
    ["research", "search", "find"].includes(tokens[3] ?? "")
  ) {
    return true;
  }
  if (first !== "use") {
    return false;
  }

  const externalSurfaceIndex = externalSurfaceAfter(tokens, 1);
  if (externalSurfaceIndex < 0) {
    return false;
  }
  const toIndex = tokens.indexOf("to");
  if (toIndex <= externalSurfaceIndex) {
    return false;
  }
  return tokens.some((token, index) => {
    if (index <= toIndex) {
      return false;
    }
    if (["search", "browse", "check", "confirm", "find", "research", "see", "verify"].includes(token)) {
      return true;
    }
    return token === "look" && tokens[index + 1] === "up";
  });
}

function externalSurfaceAfter(tokens: string[], startIndex: number) {
  let index = startIndex;
  if (["on", "using"].includes(tokens[index])) {
    index += 1;
  }
  if (tokens[index] === "the") {
    index += 1;
  }
  if (tokens[index] === "public") {
    index += 1;
  }
  return externalSearchSurfaces.has(tokens[index]) ? index : -1;
}

function hasLookUpExternalLocator(tokens: string[], startIndex: number) {
  if (tokens.slice(startIndex).includes("online")) {
    return true;
  }
  return tokens.some(
    (token, index) =>
      index >= startIndex &&
      ["on", "using"].includes(token) &&
      externalSurfaceAfter(tokens, index) >= 0,
  );
}

export function hasFreshnessLocalWebSearchIntent(content: string) {
  const normalized = content.trim();
  if (!normalized || retrospectiveSearchMentionPattern.test(normalized)) {
    return false;
  }
  return freshnessIntentPattern.test(normalized) || hasLocalizedFreshnessIntent(normalized);
}

/**
 * Private workspace data has a native/connector route. It is never a web-search
 * query, even when its wording also contains generic freshness terms.
 */
export function hasPrivateLocalDataIntent(content: string) {
  return privateLocalDataTargetPattern.test(content.trim());
}

/**
 * A clear user-authored public search directive grants one-turn authority for
 * its exact bounded query. The per-session Search control governs ambient
 * grounding; it cannot veto an explicit request and never authorizes private
 * or derived data to leave the Mac.
 */
export function isLocalWebSearchAuthorized(
  content: string,
  automatedWebGroundingEnabled: boolean,
) {
  return authorizeLocalWebSearch({
    utterance: content,
    searchControlEnabled: automatedWebGroundingEnabled,
    sources: hasPrivateLocalDataIntent(content)
      ? [{ kind: "private_local", source: "local_workspace_intent", digest: "pending" }]
      : [{ kind: "user_text" }],
  }).allowed;
}

/**
 * Source provenance is evaluated before language intent. Language heuristics
 * may select a convenient local route, but can never make private or
 * unclassified derived data public.
 */
export function authorizeLocalWebSearch(
  input: SearchAuthorizationInput,
): SearchAuthorizationDecision {
  if (input.sources.some((source) => source.kind === "private_local")) {
    return { allowed: false, reason: "private_source" };
  }
  if (input.sources.some((source) => source.kind === "unknown_derived")) {
    return { allowed: false, reason: "unknown_derived_source" };
  }
  const utterance = input.utterance.trim();
  if (!utterance) {
    return { allowed: false, reason: "not_explicit" };
  }
  if (hasExplicitLocalWebSearchIntent(utterance)) {
    const query = extractLocalWebSearchQuery(utterance);
    if (!query) {
      return { allowed: false, reason: "weak_query" };
    }
    return { allowed: true, reason: "explicit_public_search", query };
  }
  if (input.searchControlEnabled && hasFreshnessLocalWebSearchIntent(utterance)) {
    // Ambient grounding is already bounded to the immutable accepted user
    // utterance. Preserve its wording exactly apart from whitespace so native
    // ownership and audit evidence do not depend on a lossy punctuation edit.
    const query = normalizeAmbientSearchQuery(utterance);
    if (!query || isWeakSearchQuery(query)) {
      return { allowed: false, reason: "weak_query" };
    }
    return { allowed: true, reason: "ambient_freshness_search", query };
  }
  return {
    allowed: false,
    reason: input.searchControlEnabled ? "not_explicit" : "search_disabled",
  };
}

function normalizeAmbientSearchQuery(content: string) {
  return content.trim().replace(/\s+/g, " ");
}

export function extractLocalWebSearchQuery(content: string, context: SearchContextMessage[] = []) {
  // Context is deliberately not used to manufacture a network query. The
  // native boundary can verify only what this immutable utterance explicitly
  // authorized, so a pronoun-only follow-up must ask for a concrete topic.
  void context;
  const directQuery = explicitSearchQueryFromText(content);
  return isWeakSearchQuery(directQuery) ? "" : directQuery;
}

/**
 * Produces the smallest public query set explicitly authorized by one immutable
 * user utterance. Separate searches are emitted only when the user asks for
 * separate treatment of two named public subjects.
 */
export function extractLocalWebSearchQueries(content: string) {
  const primary = extractLocalWebSearchQuery(content);
  if (!primary) return [];

  const normalized = normalizeSearchQueryText(content);
  if (/\bsearch\s+each\s+separately\b/i.test(normalized)) {
    const releaseSubjects = normalized.match(
      /\b(?:latest\s+)?stable\s+releases?\s+of\s+(.+?)\s+and\s+(.+?)\s+from\s+(?:their\s+)?official\s+websites?\b/i,
    );
    if (releaseSubjects) {
      const left = normalizeSearchQueryText(releaseSubjects[1]);
      const right = normalizeSearchQueryText(releaseSubjects[2]);
      if (left && right) {
        if (/\brelease\s+dates?\b/i.test(normalized)) {
          return [
            `latest stable ${left} release date official website`,
            `latest stable ${right} release date official website`,
          ];
        }
        return [
          `latest stable release of ${left} official website`,
          `latest stable release of ${right} official website`,
        ];
      }
    }
  }
  return [primary];
}

export function isObjectiveBoundSearchContinuation(
  objective: string,
  query: string,
) {
  if (
    !hasExplicitLocalWebSearchIntent(objective) ||
    hasPrivateLocalDataIntent(objective) ||
    hasPrivateLocalDataIntent(query)
  ) {
    return false;
  }
  const queryTokens = significantSearchTokens(query);
  const objectiveTokens = new Set(significantSearchTokens(objective));
  if (queryTokens.length === 0) return false;
  const safeRefinementTokens = new Set([
    "date", "dates", "latest", "notes", "official", "public", "release", "releases", "stable", "website", "websites",
  ]);
  const objectiveMatches = queryTokens.filter((token) => objectiveTokens.has(token));
  if (objectiveMatches.length === 0) return false;
  return queryTokens.every((token) =>
    objectiveTokens.has(token) ||
    safeRefinementTokens.has(token) ||
    isVersionRefinementToken(token)
  );
}

type VerifiedSearchDocument = {
  title?: unknown;
  url?: unknown;
  snippet?: unknown;
  visibleText?: unknown;
};

type VerifiedSearchContext = {
  results?: unknown;
  pages?: unknown;
};

/**
 * Resolves the second half of an explicitly dependent release-research turn.
 * The version is accepted only from an HTTPS source whose host is visibly
 * bound to the named public subject. This keeps the follow-up deterministic
 * without broadening the user's one-turn search authority.
 */
export function exactVersionReleaseNotesSearchQuery(
  objective: string,
  initialQuery: string,
  contextJson: string | undefined,
) {
  if (
    !contextJson?.trim() ||
    !/\b(?:(?:that|this)(?:\s+exact)?|the\s+exact)\s+version\b/i.test(objective) ||
    !/\brelease\s+notes?\b/i.test(objective)
  ) {
    return "";
  }
  const subject = releaseSubject(initialQuery) || releaseSubject(objective);
  if (!subject) return "";

  let parsed: VerifiedSearchContext;
  try {
    parsed = JSON.parse(contextJson) as VerifiedSearchContext;
  } catch {
    return "";
  }
  const documents = [
    ...verifiedSearchDocuments(parsed.pages),
    ...verifiedSearchDocuments(parsed.results),
  ].filter((document) => officialSubjectDocument(subject, document));
  if (documents.length === 0) return "";

  const releaseIndexes = documents.filter((document) => {
    const url = typeof document.url === "string" ? document.url : "";
    const title = typeof document.title === "string" ? document.title : "";
    return /(?:\/releases\/?(?:$|[?#])|releases\.html(?:$|[?#]))/i.test(url) ||
      /\brelease\s+(?:announcements?|notes?)\b/i.test(title);
  });
  const indexVersion = releaseIndexes
    .map((document) => firstStableVersionFromDocument(subject, document))
    .find(Boolean);
  const version = indexVersion || highestStableAnnouncementVersion(subject, documents);
  if (!version) return "";

  const query = `${subject} ${version} official release notes`;
  return isObjectiveBoundSearchContinuation(objective, query) ? query : "";
}

function releaseSubject(value: string) {
  const match = value.match(
    /\b(?:latest|newest|most\s+recent)\s+(?:stable\s+)?(.+?)\s+releases?\b/i,
  );
  return normalizeSearchQueryText(match?.[1] ?? "")
    .replace(/^(?:the|a|an)\s+/i, "")
    .trim();
}

function verifiedSearchDocuments(value: unknown): VerifiedSearchDocument[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is VerifiedSearchDocument =>
        Boolean(entry) && typeof entry === "object")
    : [];
}

function officialSubjectDocument(subject: string, document: VerifiedSearchDocument) {
  if (typeof document.url !== "string") return false;
  try {
    const url = new URL(document.url);
    if (url.protocol !== "https:") return false;
    const host = url.hostname.toLocaleLowerCase("en-US").replace(/^www\./, "");
    const subjectTokens = subject
      .toLocaleLowerCase("en-US")
      .split(/[^a-z0-9]+/)
      .filter((token) => token.length >= 3 && !functionWords.has(token));
    return subjectTokens.some((token) => host.includes(token));
  } catch {
    return false;
  }
}

function firstStableVersionFromDocument(
  subject: string,
  document: VerifiedSearchDocument,
) {
  const text = documentText(document);
  const escapedSubject = escapeRegularExpression(subject);
  for (const pattern of [
    new RegExp(`\\bAnnouncing\\s+${escapedSubject}\\s+v?(\\d+\\.\\d+(?:\\.\\d+)?)\\b`, "i"),
    /\bVersion\s+v?(\d+\.\d+(?:\.\d+)?)\s*\(/i,
  ]) {
    const version = pattern.exec(text)?.[1];
    if (version) return version;
  }
  return "";
}

function highestStableAnnouncementVersion(
  subject: string,
  documents: VerifiedSearchDocument[],
) {
  const escapedSubject = escapeRegularExpression(subject);
  const announcement = new RegExp(
    `\\b(?:Announcing\\s+)?${escapedSubject}\\s+v?(\\d+\\.\\d+(?:\\.\\d+)?)\\b`,
    "gi",
  );
  const versions = documents.flatMap((document) => {
    const text = documentText(document);
    return Array.from(text.matchAll(announcement), (match) => match[1]);
  });
  return versions.sort(compareStableVersions).at(-1) ?? "";
}

function compareStableVersions(left: string, right: string) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

function documentText(document: VerifiedSearchDocument) {
  return [document.title, document.url, document.snippet, document.visibleText]
    .filter((value): value is string => typeof value === "string")
    .join("\n");
}

function escapeRegularExpression(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isVersionRefinementToken(token: string) {
  return /^v?\d+(?:\.\d+){1,3}(?:[-+][a-z0-9.-]+)?$/i.test(token);
}

function explicitSearchQueryFromText(content: string) {
  const localizedQuery = localizedExplicitSearchQuery(content);
  if (localizedQuery) {
    return normalizeSearchQueryText(localizedQuery);
  }
  const boundedResearchQuery = boundedPublicResearchQueryFromText(content);
  if (boundedResearchQuery) {
    return boundedResearchQuery;
  }
  for (const rawClause of [content, ...content.split(/[!?;\n]+|\.(?:\s+|$)/)]) {
    const clause = stripSearchCourtesyPrefix(normalizeSearchQueryText(rawClause));
    if (!clause) continue;

    const direct = clause.match(
      /^(?:search|browse|check|confirm|find|look|research|see|verify)\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s*(?:(?:for|about|on|regarding)\s+|(?:to|and)\s+(?:check|confirm|find|look(?:\s+up)?|research|search(?:\s+for)?|see|verify)\s+|(?:if|whether|that)\s+)?(.+?)(?:\.\s+(?:cite|include|provide|return|summarize|write|create|list|explain|then)\b.*)?$/i,
    );
    const useSurface = clause.match(
      /^use\s+(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s+to\s+(?:search|browse|check|confirm|find|look\s+up|research|see|verify)\s*(?:(?:for|about|on|regarding)\s+|(?:if|whether|that)\s+)?(.+?)(?:\.\s+(?:cite|include|provide|return|summarize|write|create|list|explain|then)\b.*)?$/i,
    );
    const lookUpSuffix = clause.match(
      /^look\s+(.+?)\s+up\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$/i,
    );
    const lookUpPrefix = clause.match(
      /^look\s+up\s+(.+?)\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$/i,
    );
    const goOnline = clause.match(
      /^go\s+online\s*,?\s+and\s+(?:research|search(?:\s+for)?|find)\s*:?[ ]*(.+)$/i,
    );
    const candidate = goOnline?.[1] ?? direct?.[1] ?? useSurface?.[1] ?? lookUpSuffix?.[1] ?? lookUpPrefix?.[1];
    if (candidate) {
      return stripSearchDeliveryInstruction(normalizeSearchQueryText(candidate))
        .replace(/^(?:if|whether|that)\s+/i, "")
        .replace(/,?\s+then\s+(?:check|find|look|open|read|review|search|verify)\b.*$/i, "")
        .replace(/^what\s+you\s+can\s+find\s+about\s+/i, "")
        .trim();
    }
  }
  return "";
}

function boundedPublicResearchQueryFromText(content: string) {
  if (hasPrivateLocalDataIntent(content) || externalResearchOptOutPattern.test(content)) {
    return "";
  }
  for (const rawClause of content.split(/[!?;\n]+|\.(?:\s+|$)/)) {
    const clause = stripSearchCourtesyPrefix(normalizeSearchQueryText(rawClause));
    const candidate = boundedPublicResearchPattern.exec(clause)?.[1];
    if (!candidate) continue;
    const query = normalizeSearchQueryText(candidate);
    if (query && !isWeakSearchQuery(query)) return query;
  }
  return "";
}

function stripSearchCourtesyPrefix(content: string) {
  let value = content;
  while (value) {
    const next = value
      .replace(/^(?:please|oomu)\b[,\s:]*/i, "")
      .replace(/^(?:can|could|would|will)\s+you\b[,\s:]*/i, "")
      .replace(/^i\s+(?:want|need)\s+you\s+(?:to\s+)?/i, "")
      .replace(/^go\s+ahead\s+and\s+/i, "")
      .trim();
    if (next === value) break;
    value = next;
  }
  return value;
}

function isWeakSearchQuery(query: string) {
  const normalized = normalizeSearchQueryText(query);
  if (!normalized) {
    return true;
  }
  const words = normalized.split(/\s+/);
  return words.length <= 4 && isFunctionWordPhrase(normalized);
}

function isFunctionWordPhrase(content: string) {
  return normalizeSearchQueryText(content)
    .split(/\s+/)
    .every((word) => functionWords.has(word.toLowerCase()));
}

function normalizeSearchQueryText(content: string) {
  return content
    .trim()
    .replace(/^[\s"'`]+|[\s"'`]+$/g, "")
    .replace(/^[,.:;!?-]+|[,.:;!?-]+$/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function stripSearchDeliveryInstruction(content: string) {
  return content
    .replace(
      /,?\s+and\s+(?:(?:give|provide|send|show|tell)\s+me|return(?:\s+me)?)\b.*$/i,
      "",
    )
    .trim();
}

function significantSearchTokens(content: string) {
  return normalizeSearchQueryText(content)
    .toLocaleLowerCase("en-US")
    .split(/[^a-z0-9.]+/)
    .filter((token) => token.length >= 2 && !functionWords.has(token));
}
