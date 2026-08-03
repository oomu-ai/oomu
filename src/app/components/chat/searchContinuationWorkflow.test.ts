import { describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import { createSearchContinuationState } from "./searchContinuationCoordinator";
import {
  handleSearchContinuationRequest,
  searchContinuationTurnContext,
  type SearchContinuationPendingSteer,
} from "./searchContinuationWorkflow";

describe("search continuation workflow", () => {
  it("continues from verified read-only evidence without claiming a pending action", async () => {
    const turn = createChatTurnContext({
      turnId: "turn-294",
      generationToken: "generation-294",
      sessionId: "session-294",
      agentId: "oomu",
      route: {
        providerId: "local_model",
        modelId: "gemma-4-e4b",
        dynamicRoutingEnabled: false,
        automatedWebGroundingEnabled: true,
      },
    });
    const state = createSearchContinuationState(
      turn,
      "Search the web for the official Rust release date.",
    );
    const attachment = {
      name: "verified-search.json",
      mime_type: "application/json",
      byte_count: 42,
      text: "verified evidence",
    };
    let pending: SearchContinuationPendingSteer<string, typeof attachment> | null = null;

    await handleSearchContinuationRequest<string, typeof attachment>(
      {
        query: "official Rust release date",
        blockText: "```oomu_search_request\n{}\n```",
      },
      searchContinuationTurnContext(turn, 294, state, [] as string[], 0),
      {
        isCurrent: () => true,
        runSearch: vi.fn().mockResolvedValue({
          kind: "succeeded",
          explicit: true,
          attachments: [attachment],
          debug: {
            query: "official Rust release date",
            engine: "duckduckgo",
            resultCount: 1,
            domPageCount: 0,
            headlessFallbackCount: 0,
            retrievalElapsedMs: 10,
          },
          queries: ["official Rust release date"],
          receipts: [{ digest: "a".repeat(64), invocationIndex: 1 }],
        }),
        setStatus: vi.fn(),
        setFailure: vi.fn(),
        replacePending: (_sessionId, value) => { pending = value; },
        releaseAttachments: vi.fn(),
        searchingStatus: "Searching",
        readyStatus: "Ready",
        incompleteMessage: "Incomplete",
        failureMessage: (code) => code,
      },
    );

    expect(pending).toMatchObject({
      sessionId: "session-294",
      providerId: "local_model",
      modelId: "gemma-4-e4b",
      assistantMessageId: 294,
      executableActionExpected: false,
      verifiedNativeExecutionReceipt: true,
      attachments: [attachment],
      turnContext: {
        route: {
          providerId: "local_model",
          modelId: "gemma-4-e4b",
        },
        ancestry: {
          kind: "steer",
          parentTurnId: "turn-294",
          rootTurnId: "turn-294",
        },
      },
    });
  });
});
