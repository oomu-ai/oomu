import { describe, expect, it } from "vitest";
import {
  MAX_SEARCH_CONTINUATION_MS,
  assistantTextForSearchContinuation,
  authorizeSearchContinuation,
  bindInitialSearchOutcome,
  createSearchContinuationState,
  initialSearchQueries,
  parseSearchContinuationRequest,
  recordInitialSearchQueries,
} from "./searchContinuationCoordinator";

const identity = {
  sessionId: "session-294",
  turnId: "turn-294",
  generationToken: "generation-294",
};
const objective = "Go online and research the latest stable releases of Rust and Node.js from their official websites. Search each separately, compare their release dates, and cite both official sources.";

describe("search continuation coordinator", () => {
  it("plans two bounded objective-owned searches for the acceptance prompt", () => {
    expect(initialSearchQueries(objective)).toEqual([
      "latest stable Rust release date official website",
      "latest stable Node.js release date official website",
    ]);
  });

  it("binds every continuation to lineage, objective, count, and wall time", () => {
    const started = createSearchContinuationState(identity, objective, 1_000);
    const withInitial = recordInitialSearchQueries(started, initialSearchQueries(objective));
    expect(withInitial.invocations).toBe(2);
    expect(authorizeSearchContinuation(
      withInitial,
      identity,
      "Rust official release date",
      2_000,
    ).allowed).toBe(true);
    expect(authorizeSearchContinuation(
      withInitial,
      { ...identity, turnId: "other" },
      "Rust official release date",
      2_000,
    )).toMatchObject({ allowed: false, reason: "lineage_mismatch" });
    expect(authorizeSearchContinuation(
      withInitial,
      identity,
      "private calendar records",
      2_000,
    )).toMatchObject({ allowed: false, reason: "unauthorized_query" });
    expect(authorizeSearchContinuation(
      withInitial,
      identity,
      "Rust official release date",
      1_000 + MAX_SEARCH_CONTINUATION_MS + 1,
    )).toMatchObject({ allowed: false, reason: "timeout" });
  });

  it("records a completed read-only search without inventing a pending action receipt", () => {
    const started = createSearchContinuationState(identity, objective, 1_000);
    const bound = bindInitialSearchOutcome(started, {
      kind: "succeeded",
      explicit: true,
      queries: initialSearchQueries(objective),
    });

    expect(bound.invocations).toBe(2);
    expect(bound.completedQueries).toEqual(initialSearchQueries(objective));
    expect(bound).not.toHaveProperty("requiresNativeReceipt");
  });

  it("parses one typed request without leaking its envelope", () => {
    const text = "I need one narrower source.\n```oomu_search_request\n{\"query\":\"Rust official release date\"}\n```";
    const request = parseSearchContinuationRequest(text);
    expect(request?.query).toBe("Rust official release date");
    expect(request && assistantTextForSearchContinuation(text, request)).toBe(
      "I need one narrower source.",
    );
    expect(parseSearchContinuationRequest("```oomu_search_request\n{\"query\":\"x\",\"private\":true}\n```"))
      .toBeNull();
  });
});
