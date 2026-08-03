import { invoke, isTauriRuntime } from "@/lib/invoke";
import {
  authorizeLocalWebSearch,
  exactVersionReleaseNotesSearchQuery,
  extractLocalWebSearchQuery,
  extractLocalWebSearchQueries,
  hasExplicitLocalWebSearchIntent,
  type SearchAuthorizationDecision,
  type SearchSource,
} from "@/lib/webSearchIntent";
import {
  localSearchFailureCode,
  localSearchFailureKind,
  type LocalSearchFailureCode,
  type LocalSearchFailureKind,
} from "./localSearchErrors";

type SearchResponse = {
  contextJson: string;
  degraded: boolean;
  engine: string;
  query: string;
  resultCount: number;
  receiptDigest?: string;
  invocationIndex?: number;
};

type ActivePageResponse = {
  contextJson: string;
  retrievalElapsedMs: number;
  usedHeadlessBrowser: boolean;
};

export type HeadlessSearchDebug = {
  query: string;
  engine: string;
  resultCount: number;
  domPageCount: number;
  headlessFallbackCount: number;
  retrievalElapsedMs: number;
};

export type LocalSearchAttachment = {
  name: string;
  mime_type: string;
  byte_count: number;
  text: string;
};

export type LocalSearchOutcome =
  | {
      kind: "not_requested";
      explicit: false;
      authorization: SearchAuthorizationDecision;
    }
  | {
      kind: "succeeded";
      explicit: boolean;
      attachments: LocalSearchAttachment[];
      debug: HeadlessSearchDebug;
      verifiedContextJson?: string;
      queries: string[];
      receipts: Array<{ digest: string; invocationIndex: number }>;
    }
  | {
      kind: LocalSearchFailureKind;
      explicit: boolean;
      errorCode: LocalSearchFailureCode;
      authorization?: SearchAuthorizationDecision;
    };

type SovereignSearchResponse = SearchResponse & {
  retrievalElapsedMs: number;
  domPageCount: number;
  headlessFallbackCount: number;
  errorCode?: string;
};

type Translate = (key: string) => string;

export type LocalSearchRequestOptions = {
  activePageAvailable?: boolean;
  searchQuery?: string;
  targetSessionId?: string;
  sources?: SearchSource[];
  modId?: string;
};

export function fetchLocalSearchForTurn(
  query: string,
  owner: { sessionId: string; turnId: string; generationToken: string },
  options: LocalSearchRequestOptions,
  runtime: {
    searchControlEnabled: boolean;
    messages: Array<{ role?: string; content: string }>;
    translate: Translate;
    setStatus: (sessionId: string, status: string) => void;
    setDebug: (sessionId: string, debug: HeadlessSearchDebug) => void;
  },
) {
  const targetSessionId = options.targetSessionId ?? owner.sessionId;
  return fetchLocalSearchAttachments({
    query,
    originTurnId: owner.turnId,
    originGenerationToken: owner.generationToken,
    searchControlEnabled: runtime.searchControlEnabled,
    activePageAvailable: options.activePageAvailable,
    searchQuery: options.searchQuery,
    fallbackQuery: extractLocalWebSearchQuery(query, runtime.messages),
    targetSessionId,
    sources: options.sources ?? [{ kind: "user_text" }],
    modId: options.modId,
    translate: runtime.translate,
    setStatus: (status) => runtime.setStatus(targetSessionId, status),
    setDebug: (debug) => runtime.setDebug(targetSessionId, debug),
  });
}

type LocalSearchRuntimeInput = {
  query: string;
  originTurnId: string;
  originGenerationToken: string;
  searchControlEnabled: boolean;
  activePageAvailable?: boolean;
  searchQuery?: string;
  fallbackQuery?: string;
  targetSessionId?: string;
  sources?: SearchSource[];
  modId?: string;
  translate: Translate;
  setStatus: (status: string) => void;
  setDebug: (debug: HeadlessSearchDebug) => void;
};

export function shouldUseLocalWebSearch(
  input: {
    utterance: string;
    searchControlEnabled: boolean;
    sources: SearchSource[];
  },
): SearchAuthorizationDecision {
  const decision = authorizeLocalWebSearch(input);
  return decision;
}

export function isDirectModNetworkRequest(
  modId: string | undefined,
  sources: SearchSource[],
) {
  return Boolean(modId?.trim()) && sources.every((source) => source.kind === "user_text");
}

export function localSearchAttachment(response: SearchResponse, index = 0) {
  const text = response.contextJson.trim();
  if (response.degraded || response.resultCount === 0 || !text || text === "[]") return null;
  const nativeReceipt = response.receiptDigest && response.invocationIndex
      ? [
        `Native-Receipt: ${response.receiptDigest}`,
        `Invocation-Index: ${response.invocationIndex}`,
        `Result-Count: ${response.resultCount}`,
      ]
    : [];
  const attachmentText = [
    "Local Web Search Context",
    `Query: ${response.query}`,
    `Engine: ${response.engine}`,
    ...nativeReceipt,
    "Isolation: keyless public search plus sanitized DOM streaming; no API key; no persistent cookies; no proxy environment; no visible browser panel.",
    "",
    text,
  ].join("\n");
  return {
    name: index > 0 ? `local_web_search_${index + 1}.md` : "local_web_search.md",
    mime_type: "text/markdown",
    byte_count: new TextEncoder().encode(attachmentText).length,
    text: attachmentText,
  };
}

export function localSearchOutcomeStopsInference(outcome: LocalSearchOutcome) {
  return outcome.kind !== "succeeded" && outcome.kind !== "not_requested" && outcome.explicit;
}

export function releaseSucceededLocalSearchOutcome(
  outcome: LocalSearchOutcome | null,
  release: (attachments: LocalSearchAttachment[]) => void,
) {
  if (outcome?.kind === "succeeded") release(outcome.attachments);
}

export function incorporateSucceededLocalSearch<T>(
  current: T[],
  outcome: LocalSearchOutcome | null,
  applied: (attachments: T[]) => void,
) {
  if (outcome?.kind !== "succeeded") return current;
  const next = [...current, ...outcome.attachments] as T[];
  applied(next);
  return next;
}

function failedSearchOutcome(
  errorCode: string | undefined,
  explicit: boolean,
  authorization?: SearchAuthorizationDecision,
): LocalSearchOutcome {
  const stableCode = localSearchFailureCode({ code: errorCode });
  return {
    kind: localSearchFailureKind(stableCode),
    explicit,
    errorCode: stableCode,
    authorization,
  };
}

export function shouldReadActivePage(
  utterance: string,
  activePageAvailable: boolean,
  searchControlEnabled: boolean,
) {
  if (!activePageAvailable || !searchControlEnabled) return false;
  const normalized = utterance.trim().toLowerCase();
  const hasPageReference = [
    "active page",
    "current page",
    "this page",
    "open page",
    "active webpage",
    "current webpage",
    "this webpage",
  ].some((phrase) => normalized.includes(phrase));
  const hasReadIntent = [
    "read",
    "summarize",
    "inspect",
    "extract",
    "review",
    "what is on",
    "what's on",
  ].some((phrase) => normalized.includes(phrase));
  return hasPageReference && hasReadIntent;
}

export function activePageAttachment(response: ActivePageResponse) {
  const text = response.contextJson.trim();
  if (!text || text === "{}") return null;
  const attachmentText = [
    "Active Web Page Context",
    "Isolation: sanitized DOM from the user-authorized incognito browser view; scripts, styles, frames, navigation, promotions, and hidden content removed.",
    "",
    text,
  ].join("\n");
  return {
    name: "active_web_page.md",
    mime_type: "text/markdown",
    byte_count: new TextEncoder().encode(attachmentText).length,
    text: attachmentText,
  };
}

async function fetchActivePageOutcome(
  input: LocalSearchRuntimeInput,
  requested: boolean,
): Promise<LocalSearchOutcome | null> {
  if (!requested) return null;
  if (!isTauriRuntime) {
    input.setStatus(input.translate("chat.status.local_search_skipped"));
    return failedSearchOutcome("search_unavailable", true);
  }

  try {
    input.setStatus(input.translate("chat.status.reading_active_page"));
    const result = await invoke<ActivePageResponse>("scrape_active_page_content", {
      request: {
        originatingUtterance: input.query,
        sessionId: input.targetSessionId,
        modId: input.modId,
        originTurnId: input.originTurnId,
        originGenerationToken: input.originGenerationToken,
      },
    });
    const attachment = activePageAttachment(result);
    if (!attachment) {
      input.setStatus(input.translate("chat.status.local_search_empty"));
      return failedSearchOutcome("search_no_results", true);
    }
    const debug = {
      query: input.translate("chat.drawer.wozniak_active_page"),
      engine: "active_browser_dom",
      resultCount: 1,
      domPageCount: 1,
      headlessFallbackCount: result.usedHeadlessBrowser ? 1 : 0,
      retrievalElapsedMs: result.retrievalElapsedMs,
    };
    input.setDebug(debug);
    input.setStatus(input.translate("chat.status.active_page_ready"));
    return {
      kind: "succeeded",
      explicit: true,
      attachments: [attachment],
      debug,
      queries: [input.query],
      receipts: [],
    };
  } catch (error) {
    input.setStatus(input.translate("chat.status.local_search_skipped"));
    return failedSearchOutcome(localSearchFailureCode(error), true);
  }
}

function searchQueryFor(
  input: LocalSearchRuntimeInput,
  decision: SearchAuthorizationDecision,
) {
  return input.searchQuery?.trim() || decision.query || input.fallbackQuery?.trim();
}

async function fetchSovereignSearchOutcome(
  input: LocalSearchRuntimeInput,
  decision: SearchAuthorizationDecision,
  explicitRequest: boolean,
  query: string,
  attachmentIndex = 0,
): Promise<LocalSearchOutcome> {
  let unlisten: (() => void) | undefined;
  try {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<{
        sessionId: string;
        turnId: string;
        generationToken: string;
        stage: string;
      }>("sovereign-search://progress", ({ payload }) => {
        if (
          payload.sessionId !== input.targetSessionId ||
          payload.turnId !== input.originTurnId ||
          payload.generationToken !== input.originGenerationToken
        ) return;
        if (payload.stage === "search_started") {
          input.setStatus(input.translate(
            input.modId ? "chat.status.headless_searching" : "chat.status.searching_web",
          ));
        } else if (payload.stage === "evidence_received") {
          input.setStatus(input.translate(
            input.modId ? "chat.status.headless_search_ready" : "chat.status.local_search_ready",
          ));
        }
      });
    } catch {
      // The invoke result remains authoritative if the event bridge is unavailable.
    }
    input.setStatus(input.translate(
      input.modId ? "chat.status.headless_searching" : "chat.status.searching_web",
    ));
    const result = await invoke<SovereignSearchResponse>("sovereign_duckduckgo_search", {
      request: {
        query,
        originatingUtterance: input.query,
        maxResults: 5,
        sessionId: input.targetSessionId,
        modId: input.modId,
        originTurnId: input.originTurnId,
        originGenerationToken: input.originGenerationToken,
      },
    });
    if (result.degraded) {
      input.setStatus(input.translate("chat.status.local_search_skipped"));
      return failedSearchOutcome(result.errorCode, explicitRequest, decision);
    }
    const attachment = localSearchAttachment(result, attachmentIndex);
    if (!attachment) {
      input.setStatus(input.translate("chat.status.local_search_empty"));
      return failedSearchOutcome("search_no_results", explicitRequest, decision);
    }
    const debug = {
      query: result.query,
      engine: result.engine,
      resultCount: result.resultCount,
      domPageCount: result.domPageCount ?? 0,
      headlessFallbackCount: result.headlessFallbackCount ?? 0,
      retrievalElapsedMs: result.retrievalElapsedMs,
    };
    input.setDebug(debug);
    input.setStatus(input.translate(
      input.modId ? "chat.status.headless_search_ready" : "chat.status.local_search_ready",
    ));
    return {
      kind: "succeeded",
      explicit: explicitRequest,
      attachments: [attachment],
      debug,
      verifiedContextJson: result.contextJson,
      queries: [query],
      receipts: result.receiptDigest && result.invocationIndex
        ? [{ digest: result.receiptDigest, invocationIndex: result.invocationIndex }]
        : [],
    };
  } catch (error) {
    input.setStatus(input.translate("chat.status.local_search_skipped"));
    return failedSearchOutcome(localSearchFailureCode(error), explicitRequest, decision);
  } finally {
    unlisten?.();
  }
}

export async function fetchLocalSearchAttachments(
  input: LocalSearchRuntimeInput,
): Promise<LocalSearchOutcome> {
  const sources = input.sources ?? [{ kind: "user_text" as const }];
  const directModNetworkRequest = isDirectModNetworkRequest(input.modId, sources);
  const activePageRequested = shouldReadActivePage(
    input.query,
    Boolean(input.activePageAvailable),
    input.searchControlEnabled || directModNetworkRequest,
  );
  const activePageOutcome = await fetchActivePageOutcome(input, activePageRequested);
  if (activePageOutcome) return activePageOutcome;

  const searchDecision = shouldUseLocalWebSearch({
    utterance: input.query,
    searchControlEnabled: input.searchControlEnabled,
    sources,
  });
  const explicitRequest =
    directModNetworkRequest || hasExplicitLocalWebSearchIntent(input.query);
  if (!searchDecision.allowed && !directModNetworkRequest) {
    if (
      explicitRequest &&
      (
        searchDecision.reason === "private_source" ||
        searchDecision.reason === "unknown_derived_source"
      )
    ) {
      input.setStatus(input.translate("chat.status.private_search_blocked"));
    }
    return explicitRequest
      ? failedSearchOutcome(
          searchDecision.reason === "weak_query"
            ? "search_query_invalid"
            : "search_not_authorized",
          true,
          searchDecision,
        )
      : { kind: "not_requested", explicit: false, authorization: searchDecision };
  }
  if (!isTauriRuntime) {
    input.setStatus(input.translate("chat.status.local_search_skipped"));
    return failedSearchOutcome("search_unavailable", explicitRequest, searchDecision);
  }

  const requestedQueries = input.searchQuery?.trim()
    ? [input.searchQuery.trim()]
    : extractLocalWebSearchQueries(input.query);
  const searchQueries = (requestedQueries.length > 0
    ? requestedQueries
    : [searchQueryFor(input, searchDecision)])
    .filter((query): query is string => Boolean(query?.trim()))
    .slice(0, 3);
  if (searchQueries.length === 0) {
    input.setStatus(input.translate("chat.status.local_search_needs_topic"));
    return failedSearchOutcome("search_query_invalid", explicitRequest, searchDecision);
  }
  const successful: Extract<LocalSearchOutcome, { kind: "succeeded" }>[] = [];
  for (const [index, searchQuery] of searchQueries.entries()) {
    const outcome = await fetchSovereignSearchOutcome(
      input,
      searchDecision,
      explicitRequest,
      searchQuery,
      index,
    );
    if (outcome.kind !== "succeeded") return outcome;
    successful.push(outcome);
  }
  if (successful.length === 1 && searchQueries.length < 3) {
    const followUpQuery = exactVersionReleaseNotesSearchQuery(
      input.query,
      searchQueries[0],
      successful[0].verifiedContextJson,
    );
    if (followUpQuery && !searchQueries.includes(followUpQuery)) {
      const followUp = await fetchSovereignSearchOutcome(
        input,
        searchDecision,
        explicitRequest,
        followUpQuery,
        successful.length,
      );
      if (followUp.kind !== "succeeded") return followUp;
      successful.push(followUp);
    }
  }
  const last = successful.at(-1)!;
  return {
    kind: "succeeded",
    explicit: explicitRequest,
    attachments: successful.flatMap((outcome) => outcome.attachments),
    debug: {
      ...last.debug,
      resultCount: successful.reduce((sum, outcome) => sum + outcome.debug.resultCount, 0),
      domPageCount: successful.reduce((sum, outcome) => sum + outcome.debug.domPageCount, 0),
      headlessFallbackCount: successful.reduce(
        (sum, outcome) => sum + outcome.debug.headlessFallbackCount,
        0,
      ),
      retrievalElapsedMs: successful.reduce(
        (sum, outcome) => sum + outcome.debug.retrievalElapsedMs,
        0,
      ),
    },
    verifiedContextJson: successful.length === 1 ? last.verifiedContextJson : undefined,
    queries: successful.flatMap((outcome) => outcome.queries),
    receipts: successful.flatMap((outcome) => outcome.receipts),
  };
}
