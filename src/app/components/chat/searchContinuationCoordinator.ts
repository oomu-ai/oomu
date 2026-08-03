import {
  extractLocalWebSearchQueries,
  isObjectiveBoundSearchContinuation,
} from "@/lib/webSearchIntent";

export const MAX_SEARCH_CONTINUATION_CYCLES = 4;
export const MAX_SEARCH_INVOCATIONS = 3;
export const MAX_SEARCH_CONTINUATION_MS = 90_000;

export type SearchContinuationIdentity = {
  sessionId: string;
  turnId: string;
  generationToken: string;
};

export type SearchContinuationState = SearchContinuationIdentity & {
  objective: string;
  startedAtMs: number;
  cycles: number;
  invocations: number;
  completedQueries: string[];
  terminal: boolean;
};

export type SearchContinuationDenial =
  | "cancelled"
  | "cycle_budget_exhausted"
  | "duplicate_query"
  | "lineage_mismatch"
  | "search_budget_exhausted"
  | "timeout"
  | "unauthorized_query";

export type ParsedSearchContinuationRequest = {
  query: string;
  blockText: string;
};

const searchContinuationFence =
  /```[ \t]*(?:json[ \t]+)?oomu_search_request[^\n]*\n([\s\S]*?)```/i;

export function createSearchContinuationState(
  identity: SearchContinuationIdentity,
  objective: string,
  nowMs = Date.now(),
): SearchContinuationState {
  return {
    ...identity,
    objective: objective.trim(),
    startedAtMs: nowMs,
    cycles: 0,
    invocations: 0,
    completedQueries: [],
    terminal: false,
  };
}

export function initialSearchQueries(objective: string) {
  return extractLocalWebSearchQueries(objective).slice(0, MAX_SEARCH_INVOCATIONS);
}

export function authorizeSearchContinuation(
  state: SearchContinuationState,
  identity: SearchContinuationIdentity,
  query: string,
  nowMs = Date.now(),
): { allowed: true; state: SearchContinuationState } | { allowed: false; reason: SearchContinuationDenial; state: SearchContinuationState } {
  if (
    state.sessionId !== identity.sessionId ||
    state.turnId !== identity.turnId ||
    state.generationToken !== identity.generationToken
  ) {
    return { allowed: false, reason: "lineage_mismatch", state: { ...state, terminal: true } };
  }
  if (state.terminal) {
    return { allowed: false, reason: "cancelled", state };
  }
  if (nowMs - state.startedAtMs > MAX_SEARCH_CONTINUATION_MS) {
    return { allowed: false, reason: "timeout", state: { ...state, terminal: true } };
  }
  if (state.cycles >= MAX_SEARCH_CONTINUATION_CYCLES) {
    return { allowed: false, reason: "cycle_budget_exhausted", state: { ...state, terminal: true } };
  }
  if (state.invocations >= MAX_SEARCH_INVOCATIONS) {
    return { allowed: false, reason: "search_budget_exhausted", state: { ...state, terminal: true } };
  }
  const normalizedQuery = query.trim().replace(/\s+/g, " ");
  if (!isObjectiveBoundSearchContinuation(state.objective, normalizedQuery)) {
    return { allowed: false, reason: "unauthorized_query", state: { ...state, terminal: true } };
  }
  if (state.completedQueries.some((candidate) => candidate.toLocaleLowerCase("en-US") === normalizedQuery.toLocaleLowerCase("en-US"))) {
    return { allowed: false, reason: "duplicate_query", state: { ...state, terminal: true } };
  }
  return {
    allowed: true,
    state: {
      ...state,
      cycles: state.cycles + 1,
      invocations: state.invocations + 1,
      completedQueries: [...state.completedQueries, normalizedQuery],
    },
  };
}

export function recordInitialSearchQueries(
  state: SearchContinuationState,
  queries: string[],
) {
  return queries.reduce((current, query) => {
    const authorization = authorizeSearchContinuation(
      current,
      current,
      query,
      current.startedAtMs,
    );
    return authorization.allowed ? authorization.state : current;
  }, state);
}

export function bindInitialSearchOutcome(
  state: SearchContinuationState,
  outcome: { kind: string; explicit?: boolean; queries?: string[] } | null,
) {
  return outcome?.kind === "succeeded"
    ? recordInitialSearchQueries(state, outcome.queries ?? [])
    : state;
}

export function parseSearchContinuationRequest(
  text: string,
): ParsedSearchContinuationRequest | null {
  const match = searchContinuationFence.exec(text);
  if (!match) return null;
  try {
    const parsed = JSON.parse((match[1] ?? "").trim()) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const record = parsed as Record<string, unknown>;
    const query = typeof record.query === "string" ? record.query.trim() : "";
    if (!query || Object.keys(record).some((key) => key !== "query")) return null;
    return { query, blockText: match[0] };
  } catch {
    return null;
  }
}

export function assistantTextForSearchContinuation(
  assistantText: string,
  request: ParsedSearchContinuationRequest,
) {
  return assistantText.replace(request.blockText, "").trim();
}
