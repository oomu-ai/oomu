import { useMemo } from "react";
import {
  authorizeLocalWebSearch,
  extractLocalWebSearchQuery,
  hasPrivateLocalDataIntent,
  type SearchSource,
} from "@/lib/webSearchIntent";
import { projectBrowserControlEnvelope } from "./browserControlEnvelope";

type VerticalTemplateSectionKey = "clientProfile" | "resolutionPaths" | "experienceChecks";

export type BrowserRouteMessage = {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
};

export type VerticalTemplateSection = {
  key: VerticalTemplateSectionKey;
  label: string;
  content: string;
  present: boolean;
};

export type VerticalTemplateParseResult = {
  isVertical: boolean;
  sections: VerticalTemplateSection[];
  fallbackContent: string;
  completionRatio: number;
  missingSections: string[];
  rawContent: string;
};

export type VerticalTemplateRoute = {
  messageId: number;
  parsed: VerticalTemplateParseResult;
};

type OomuSplitViewDirective = {
  blockText: string;
  modId: string;
  action: string;
  url: string | null;
  reason: string | null;
};

type BrowserDirectiveGrant = {
  modId: string;
  allowedHosts?: readonly string[];
};

type LocalizedModText = Record<string, string>;

type InstalledModCommandRecord = {
  trigger: string;
  description?: LocalizedModText | string | null;
  public_network?: boolean;
  context_url_templates?: string[] | null;
};

export type InstalledModCommandSource = {
  id: string;
  name: string;
  isActive: boolean;
  endpoints?: string[] | null;
  commands?: InstalledModCommandRecord[] | null;
};

export type BrowserSplitRoute = {
  messageId: number;
  sessionId?: string | null;
  modId: string;
  action: string;
  url: string;
  reason: string | null;
  rawDirective: string;
};

export type AuthorizedBrowserResearchFallback = {
  originatingUserMessageId: number;
  originatingUtterance: string;
  query: string;
};

export const BROWSER_SPLIT_MOD_ID = "ai.eldris.mods.browser";
const GOOGLE_SEARCH_URL = "https://www.google.com/search";
const GLOBAL_BROWSER_NAVIGATION_SCOPE = "__global_browser_navigation__";

const verticalTemplateSectionLabels: Record<VerticalTemplateSectionKey, string> = {
  clientProfile: "Client Profile",
  resolutionPaths: "Resolution Paths",
  experienceChecks: "Experience Checks",
};

const verticalTemplateSectionOrder: VerticalTemplateSectionKey[] = [
  "clientProfile",
  "resolutionPaths",
  "experienceChecks",
];

export function localizedModText(
  value: LocalizedModText | string | null | undefined,
  locale: string,
) {
  if (!value) return "";
  if (typeof value === "string") return value;
  return value[locale] ?? value[locale.split("-")[0]] ?? value["en-US"] ?? "";
}

export function firstSlashTrigger(message: string) {
  const trimmed = message.trim();
  return trimmed.startsWith("/") ? trimmed.split(/\s+/, 1)[0] ?? "" : "";
}

export function mergeBrowserDirectiveGrants(
  ...groups: ReadonlyArray<readonly BrowserDirectiveGrant[]>
) {
  const grants = new Map<string, BrowserDirectiveGrant>();
  for (const group of groups) {
    for (const grant of group) {
      const modId = grant.modId.trim().toLowerCase();
      if (modId) grants.set(modId, grant);
    }
  }
  return Array.from(grants.values());
}

function usableModBrowserEndpoints(mod: InstalledModCommandSource) {
  return (mod.endpoints ?? [])
    .map((endpoint) => endpoint.trim())
    .filter((endpoint) => endpoint && endpoint.toLowerCase() !== "none declared");
}

export function headlessModSearchForMessage(
  installedMods: InstalledModCommandSource[],
  message: string,
) {
  const trigger = firstSlashTrigger(message);
  const argumentsText = message.trim().slice(trigger.length).trim();
  if (!trigger || !argumentsText) return null;
  const mod = installedMods.find(
    (candidate) =>
      candidate.isActive &&
      usableModBrowserEndpoints(candidate).length > 0 &&
      (candidate.commands ?? []).some(
        (command) => command.trigger.trim().toLowerCase() === trigger.toLowerCase(),
      ),
  );
  const command = mod?.commands?.find(
    (candidate) => candidate.trigger.trim().toLowerCase() === trigger.toLowerCase(),
  );
  const declaresPublicNetwork = command?.public_network === true ||
    (command?.context_url_templates ?? []).some((template) => template.trim());
  if (!mod || !command || !declaresPublicNetwork) {
    return null;
  }
  return {
    modId: mod.id,
    query: `${trigger.slice(1).replace(/[-_]+/g, " ")} ${argumentsText}`
      .split(/\s+/)
      .join(" "),
  };
}

export function browserDirectiveGrantsForMessage(
  installedMods: InstalledModCommandSource[],
  message: string,
  activeRoute?: BrowserSplitRoute | null,
) {
  const trigger = firstSlashTrigger(message).toLowerCase();
  const explicitNetworkMod = trigger
    ? installedMods.find(
        (mod) =>
          mod.isActive &&
          usableModBrowserEndpoints(mod).length > 0 &&
          (mod.commands ?? []).some(
            (command) => command.trigger.trim().toLowerCase() === trigger,
          ),
      )
    : null;
  // Public-network mod commands are always headless. A slash command owned by
  // one of these mods cannot borrow the visible Browser mod, even when its
  // prose happens to contain words such as "open" or "browser".
  if (explicitNetworkMod) {
    return [];
  }
  const coreAuthorized =
    hasExplicitBrowserNavigationIntent(message) ||
    activeRoute?.modId.trim().toLowerCase() === BROWSER_SPLIT_MOD_ID;
  return coreAuthorized
    ? [{ modId: BROWSER_SPLIT_MOD_ID }]
    : [];
}

function normalizeVerticalHeader(value: string) {
  return value
    .trim()
    .replace(/^#{1,6}\s*/, "")
    .replace(/^[*-]\s*/, "")
    .replace(/[:：]+$/, "")
    .replace(/\s+/g, " ")
    .toLowerCase();
}

function isVerticalHeaderCandidate(line: string) {
  const trimmed = line.trim();
  if (!trimmed) {
    return false;
  }
  if (/^#{1,6}\s*\S/.test(trimmed)) {
    return true;
  }
  const letters = trimmed.replace(/[^A-Za-z]/g, "");
  return letters.length >= 8 && trimmed === trimmed.toUpperCase();
}

function classifyVerticalTemplateHeader(line: string): VerticalTemplateSectionKey | null {
  if (!isVerticalHeaderCandidate(line)) {
    return null;
  }
  const normalized = normalizeVerticalHeader(line);
  if (
    normalized.includes("client profile state") ||
    normalized.includes("customer profile state") ||
    (normalized.includes("client") && normalized.includes("state")) ||
    (normalized.includes("customer") && normalized.includes("state"))
  ) {
    return "clientProfile";
  }
  if (
    normalized.includes("recommended resolution paths") ||
    normalized.includes("resolution path") ||
    (normalized.includes("recommend") && normalized.includes("resolution"))
  ) {
    return "resolutionPaths";
  }
  if (
    normalized.includes("experience enhancement checks") ||
    normalized.includes("experience check") ||
    normalized.includes("empathy checklist") ||
    (normalized.includes("tone") && normalized.includes("pitfall"))
  ) {
    return "experienceChecks";
  }
  return null;
}

function emptyVerticalTemplateParse(rawContent: string): VerticalTemplateParseResult {
  return {
    isVertical: false,
    sections: verticalTemplateSectionOrder.map((key) => ({
      key,
      label: verticalTemplateSectionLabels[key],
      content: "",
      present: false,
    })),
    fallbackContent: "",
    completionRatio: 0,
    missingSections: verticalTemplateSectionOrder.map((key) => verticalTemplateSectionLabels[key]),
    rawContent,
  };
}

export function parseVerticalTemplatePayload(content: string): VerticalTemplateParseResult {
  const rawContent = content;
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const firstNonEmptyLine = lines.find((line) => {
    const trimmed = line.trim();
    return trimmed && !trimmed.startsWith("```");
  }) ?? "";
  const firstHeader = classifyVerticalTemplateHeader(firstNonEmptyLine);
  const sectionLines: Record<VerticalTemplateSectionKey, string[]> = {
    clientProfile: [],
    resolutionPaths: [],
    experienceChecks: [],
  };
  const seenSections = new Set<VerticalTemplateSectionKey>();
  const fallbackLines: string[] = [];
  let currentSection: VerticalTemplateSectionKey | null = null;

  for (const line of lines) {
    if (line.trim().startsWith("```")) {
      continue;
    }
    const nextSection = classifyVerticalTemplateHeader(line);
    if (nextSection) {
      currentSection = nextSection;
      seenSections.add(nextSection);
      continue;
    }
    if (currentSection) {
      sectionLines[currentSection].push(line);
    } else if (line.trim()) {
      fallbackLines.push(line);
    }
  }

  const sectionCount = seenSections.size;
  const isVertical =
    firstHeader === "clientProfile" ||
    (seenSections.has("clientProfile") && sectionCount >= 2);
  if (!isVertical) {
    return emptyVerticalTemplateParse(rawContent);
  }

  const sections = verticalTemplateSectionOrder.map((key) => {
    const sectionContent = sectionLines[key].join("\n").trim();
    return {
      key,
      label: verticalTemplateSectionLabels[key],
      content: sectionContent,
      present: seenSections.has(key),
    };
  });
  const missingSections = sections
    .filter((section) => !section.present)
    .map((section) => section.label);

  return {
    isVertical,
    sections,
    fallbackContent: fallbackLines.join("\n").trim(),
    completionRatio: (sections.length - missingSections.length) / sections.length,
    missingSections,
    rawContent,
  };
}

export function latestVerticalTemplateRoute(
  messages: ReadonlyArray<BrowserRouteMessage>,
): VerticalTemplateRoute | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== "assistant") {
      continue;
    }
    const parsed = parseVerticalTemplatePayload(message.content);
    if (parsed.isVertical) {
      return {
        messageId: message.id,
        parsed,
      };
    }
  }
  return null;
}

export function verticalTemplateMessageIds(messages: ReadonlyArray<BrowserRouteMessage>) {
  return new Set(
    messages
      .filter(
        (message) =>
          message.role === "assistant" &&
          parseVerticalTemplatePayload(message.content).isVertical,
      )
      .map((message) => message.id),
  );
}

export function useVerticalTemplateParser(messages: ReadonlyArray<BrowserRouteMessage>) {
  return useMemo(() => latestVerticalTemplateRoute(messages), [messages]);
}

function splitViewBlockPattern() {
  return /<\s*OomuSplitView\b[^>]*>[\s\S]*?<\s*\/\s*OomuSplitView\s*>/gi;
}

function splitViewTagPattern() {
  return /<\s*\/?\s*OomuSplitView\b[^>]*>/gi;
}

function decodeSplitViewValue(value: string) {
  return value
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, "\"")
    .replace(/&#39;/g, "'")
    .trim();
}

function splitViewTagValue(block: string, tagName: string) {
  const match = new RegExp(
    `<\\s*${tagName}\\b[^>]*>([\\s\\S]*?)<\\s*\\/\\s*${tagName}\\s*>`,
    "i",
  ).exec(block);
  return match?.[1] ? decodeSplitViewValue(match[1]) : null;
}

function normalizeBrowserSplitUrl(value: string | null) {
  const trimmed = value?.trim() ?? "";
  if (!trimmed || /\s/.test(trimmed)) {
    return null;
  }
  const candidate = /^[a-z][a-z0-9+.-]*:/i.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;

  try {
    const url = new URL(candidate);
    if (url.protocol !== "https:" && url.protocol !== "http:") {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}

function normalizedDirectiveModId(value: string) {
  return value.trim().toLowerCase();
}

function directiveMatchesGrant(
  directive: OomuSplitViewDirective,
  grants: readonly BrowserDirectiveGrant[],
) {
  if (!directive.url) {
    return false;
  }
  // Only the dedicated Browser mod owns visible navigation. Capability mods
  // may retrieve allowlisted public context, but can never render a webview.
  if (normalizedDirectiveModId(directive.modId) !== BROWSER_SPLIT_MOD_ID) {
    return false;
  }
  const grant = grants.find(
    (candidate) =>
      normalizedDirectiveModId(candidate.modId) ===
      normalizedDirectiveModId(directive.modId),
  );
  return Boolean(grant);
}

export function browserNavigationScope(sessionId?: string | null) {
  return sessionId?.trim() || GLOBAL_BROWSER_NAVIGATION_SCOPE;
}

export function normalizedBrowserNavigationKey(url: string | null | undefined) {
  return normalizeBrowserSplitUrl(url ?? null);
}

export function browserFeedbackIndicatesFailedNavigation(content: string) {
  const normalized = content.trim().toLowerCase().replace(/\s+/g, " ");
  if (!normalized) {
    return false;
  }
  const referencesPage =
    /\b(that|this|current|opened|loaded|browser|page|site|url|link|source|result)\b/.test(normalized);
  const failureSignal =
    /\b(incorrect|wrong|bad|broken|blank|dead|invalid|irrelevant|unavailable|not\s+loading|doesn'?t\s+load|won'?t\s+load|404|not\s+found)\b/.test(normalized) ||
    /\bnot\s+(?:the\s+)?(?:right|correct)\s+(?:page|site|url|link|source|result)\b/.test(normalized) ||
    /\b(?:isn'?t|is\s+not|doesn'?t|does\s+not)\b.{0,80}\b(?:on|show|contain|have)\b.{0,80}\b(?:page|site|browser|schedule|score|result|source)\b/.test(normalized) ||
    /\bi\s+don'?t\s+think\b.{0,100}\b(?:page|site|browser|url|link|source|result)\b/.test(normalized);
  return referencesPage && failureSignal;
}

export function reportBrowserNavigationFailure(
  failure: { url: string; sessionId?: string | null } | null,
  register: (url: string, sessionId?: string | null) => void,
  reportStatus: () => void,
) {
  if (!failure) return;
  register(failure.url, failure.sessionId);
  reportStatus();
}

export function browserNavigationBlockPayload(url: string) {
  return {
    status: "navigation_blocked",
    reason: "The user marked this URL incorrect. Do not reopen it or search automatically. Ask for an explicit destination, or ask the user to enable Search and request an external search.",
    url,
    suggested_action: "request_explicit_search",
  } as const;
}

export function browserNavigationBlockedNotice(
  t: (key: string) => string,
) {
  return {
    message: t("chat.browser.navigation_blocked"),
    status: t("chat.browser.navigation_blocked_status"),
  };
}

export function browserNavigationIsBlacklisted(
  url: string,
  sessionId: string | null | undefined,
  failedNavigationUrls: ReadonlyMap<string, ReadonlySet<string>>,
) {
  const key = normalizedBrowserNavigationKey(url);
  if (!key) {
    return false;
  }
  return Boolean(
    failedNavigationUrls.get(browserNavigationScope(sessionId))?.has(key) ||
      failedNavigationUrls.get(GLOBAL_BROWSER_NAVIGATION_SCOPE)?.has(key),
  );
}

export function browserSearchFallbackQuery(
  content: string,
  context: ReadonlyArray<BrowserRouteMessage>,
  route: BrowserSplitRoute | null | undefined,
) {
  // A rejected page never authorizes a second network destination. Only a
  // fresh, explicit search request in this immutable utterance may supply one.
  void context;
  void route;
  return browserFeedbackIndicatesFailedNavigation(content)
    ? ""
    : extractLocalWebSearchQuery(content);
}

function parseOomuSplitViewDirectives(content: string): OomuSplitViewDirective[] {
  return Array.from(content.matchAll(splitViewBlockPattern())).map((match) => {
    const blockText = match[0];
    const modId =
      splitViewTagValue(blockText, "mod_id") ??
      splitViewTagValue(blockText, "modId") ??
      "";
    const action = splitViewTagValue(blockText, "action") ?? "";
    const reason = splitViewTagValue(blockText, "reason");
    return {
      blockText,
      modId,
      action: action.trim().toUpperCase(),
      url: normalizeBrowserSplitUrl(splitViewTagValue(blockText, "url")),
      reason,
    };
  });
}

export function parseBrowserSplitViewPayload(
  content: string,
  grants: readonly BrowserDirectiveGrant[] = [{ modId: BROWSER_SPLIT_MOD_ID }],
): OomuSplitViewDirective | null {
  return (
    parseOomuSplitViewDirectives(content).find(
      (directive) =>
        directive.action === "NAVIGATE" &&
        directiveMatchesGrant(directive, grants),
    ) ?? null
  );
}

export function activateAuthorizedBrowserDirective(
  content: string,
  messageId: number,
  sessionId: string,
  grants: readonly BrowserDirectiveGrant[],
  activate: (route: BrowserSplitRoute) => void,
) {
  if (grants.length === 0) {
    return false;
  }
  const directive = parseBrowserSplitViewPayload(content, grants);
  if (!directive?.url) {
    return false;
  }
  activate({
    messageId,
    sessionId,
    modId: directive.modId,
    action: directive.action,
    url: directive.url,
    reason: directive.reason,
    rawDirective: directive.blockText,
  });
  return true;
}

function browserDirectiveGrantsForPrecedingUser(
  messages: ReadonlyArray<BrowserRouteMessage>,
  assistantIndex: number,
  resolveGrants?: (message: string) => readonly BrowserDirectiveGrant[],
) {
  for (let index = assistantIndex - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "system") {
      continue;
    }
    if (message.role === "user") {
      const resolved = resolveGrants?.(message.content) ?? [];
      if (resolved.length > 0) {
        return resolved;
      }
      return browserSplitRouteFromUserPrompt(message.content, [], message.id, "")
        ? [{ modId: BROWSER_SPLIT_MOD_ID }]
        : [];
    }
    return [];
  }
  return [];
}

export function latestBrowserSplitRoute(
  messages: ReadonlyArray<BrowserRouteMessage>,
  resolveGrants?: (message: string) => readonly BrowserDirectiveGrant[],
): BrowserSplitRoute | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== "assistant") {
      continue;
    }
    const grants = browserDirectiveGrantsForPrecedingUser(
      messages,
      index,
      resolveGrants,
    );
    const directive = parseBrowserSplitViewPayload(message.content, grants);
    if (directive?.url) {
      return {
        messageId: message.id,
        modId: directive.modId,
        action: directive.action,
        url: directive.url,
        reason: directive.reason,
        rawDirective: directive.blockText,
      };
    }
  }
  return null;
}

export function stripOomuSplitViewDirectives(content: string) {
  return projectBrowserControlEnvelope(content, true)
    .visibleText.replace(splitViewTagPattern(), "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function splitViewDirectiveMessageIds(messages: ReadonlyArray<BrowserRouteMessage>) {
  return new Set(
    messages
      .filter(
        (message) =>
          message.role === "assistant" &&
          parseOomuSplitViewDirectives(message.content).length > 0 &&
          !stripOomuSplitViewDirectives(message.content),
      )
      .map((message) => message.id),
  );
}

const browserUrlCandidatePattern =
  /\b(?:https?:\/\/|www\.)[^\s<>()]+|\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+(?:com|org|net|edu|gov|io|ai|co|dev|app|shop|store|site|info|biz|us|uk|ca|de|fr|jp|au)(?:\/[^\s<>()]*)?/i;
const browserNavigationDirectivePattern =
  /^(?:please\s+)?(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?)?(?:open|visit|navigate(?:\s+to)?|load|go\s+to|take\s+me\s+to)\b/i;
const browserResearchDirectivePattern =
  /^(?:(?:please|oomu)[,:]?\s+)*(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?)?(?:use|open)\s+(?:the\s+)?(?:secure\s+)?browser\s+(?:to|and)\s+(?:research|find|look\s+up|search(?:\s+for)?|browse)\s+(.+)$/i;
const retrospectiveBrowserQuestionPattern =
  /(?:^|\b(?:explain|tell\s+me)\s+)\b(?:why|how|when|where|what)\s+(?:(?:did|do|does|would|could|was|were|is|are)\s+)?(?:you|oomu|the\s+app)\b|^(?:did|do|does|was|were|is|are)\s+(?:you|oomu|the\s+app)\b/i;

function cleanBrowserUrlCandidate(value: string) {
  return value
    .trim()
    .replace(/[),.;!?]+$/g, "")
    .replace(/^["'`]+|["'`]+$/g, "");
}

function extractBrowserUrlFromText(content: string) {
  const match = browserUrlCandidatePattern.exec(content);
  return normalizeBrowserSplitUrl(match ? cleanBrowserUrlCandidate(match[0]) : null);
}

function googleSearchUrl(query: string) {
  const url = new URL(GOOGLE_SEARCH_URL);
  url.searchParams.set("q", query);
  return url.href;
}

/**
 * Browser UI is user-authorized, not model-authorized. Mentions of a browser,
 * including questions about something OOMU previously did, must stay in chat.
 */
export function hasExplicitBrowserNavigationIntent(content: string) {
  const normalized = content.trim();
  if (!normalized || retrospectiveBrowserQuestionPattern.test(normalized)) {
    return false;
  }
  const directUrlNavigation =
    browserNavigationDirectivePattern.test(normalized) &&
    Boolean(extractBrowserUrlFromText(normalized));
  if (directUrlNavigation) {
    return true;
  }

  // Search wording is handled by the separate, visible Search consent control.
  // Only an explicit request to use the browser may open the browser panel.
  return (
    !hasPrivateLocalDataIntent(normalized) &&
    browserResearchDirectivePattern.test(normalized)
  );
}

function browserResearchQuery(content: string) {
  return browserResearchDirectivePattern.exec(content.trim())?.[1]?.trim() ?? "";
}

function originatingUserMessageForBrowserRoute(
  messages: ReadonlyArray<BrowserRouteMessage>,
  route: BrowserSplitRoute,
) {
  const routeIndex = messages.findIndex((message) => message.id === route.messageId);
  if (routeIndex < 0) return null;
  const routeMessage = messages[routeIndex];
  if (routeMessage.role === "user") return routeMessage;
  if (routeMessage.role !== "assistant") return null;

  for (let index = routeIndex - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "system") continue;
    return message.role === "user" ? message : null;
  }
  return null;
}

/**
 * A failed visible-browser route may borrow the headless search path only from
 * the exact immutable user turn that authorized public research. Route reason
 * and raw model markup are deliberately ignored: neither is user authority.
 */
export function authorizedBrowserResearchFallback(
  messages: ReadonlyArray<BrowserRouteMessage>,
  route: BrowserSplitRoute,
): AuthorizedBrowserResearchFallback | null {
  const origin = originatingUserMessageForBrowserRoute(messages, route);
  if (!origin || hasPrivateLocalDataIntent(origin.content)) return null;
  if (extractBrowserUrlFromText(origin.content)) return null;

  const browserQuery = browserResearchQuery(origin.content);
  if (!browserQuery) return null;
  const authorization = authorizeLocalWebSearch({
    utterance: `Search the web for ${browserQuery}`,
    searchControlEnabled: false,
    sources: [{ kind: "user_text" }],
  });
  if (
    !authorization.allowed ||
    authorization.reason !== "explicit_public_search" ||
    !authorization.query?.trim()
  ) {
    return null;
  }
  return {
    originatingUserMessageId: origin.id,
    originatingUtterance: origin.content,
    query: authorization.query,
  };
}

export function browserSplitRouteFromUserPrompt(
  content: string,
  _context: ReadonlyArray<BrowserRouteMessage>,
  messageId: number,
  sessionId: string,
  searchAuthorization: {
    searchControlEnabled: boolean;
    sources: SearchSource[];
  } = { searchControlEnabled: false, sources: [{ kind: "user_text" }] },
): BrowserSplitRoute | null {
  const normalized = content.trim();
  if (!hasExplicitBrowserNavigationIntent(normalized)) {
    return null;
  }

  const directUrl = extractBrowserUrlFromText(normalized);
  const query = browserResearchQuery(normalized);
  if (!directUrl && !query) {
    return null;
  }
  const searchDecision = directUrl
    ? null
    : authorizeLocalWebSearch({
        utterance: `Search Google for ${query}`,
        searchControlEnabled: searchAuthorization.searchControlEnabled,
        sources: searchAuthorization.sources,
      });
  if (!directUrl && !searchDecision?.allowed) {
    return null;
  }

  return {
    messageId,
    sessionId,
    modId: BROWSER_SPLIT_MOD_ID,
    action: "NAVIGATE",
    url: directUrl ?? googleSearchUrl(searchDecision?.query ?? query),
    reason: directUrl
      ? "Opening requested URL in the live browser panel."
      : "Opening Google search in the live browser panel.",
    rawDirective: "",
  };
}
