import { beforeEach, describe, expect, it, vi } from "vitest";
import { fetchLocalSearchAttachments } from "./localSearchContext";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauriRuntime: true,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

describe("explicit check-online execution", () => {
  beforeEach(() => {
    listenMock.mockResolvedValue(vi.fn());
    invokeMock.mockResolvedValue({
      query: "the Red Sox are playing today, July 27, 2026",
      engine: "duckduckgo_lite_static",
      resultCount: 3,
      degraded: false,
      contextJson: JSON.stringify({ pages: [{
        title: "Boston Red Sox Schedule",
        url: "https://www.mlb.com/redsox/schedule/2026-07",
        visibleText: "Official Boston Red Sox schedule for July 27, 2026.",
      }] }),
      retrievalElapsedMs: 20,
      domPageCount: 1,
      headlessFallbackCount: 0,
      receiptDigest: "receipt-red-sox",
      invocationIndex: 1,
    });
  });

  it("executes while ambient Search is off and keeps the native receipt", async () => {
    const utterance = "Check online to see if the Red Sox are playing today, July 27, 2026";
    const result = await fetchLocalSearchAttachments({
      query: utterance,
      originTurnId: "turn-red-sox",
      originGenerationToken: "generation-red-sox",
      searchControlEnabled: false,
      targetSessionId: "session-red-sox",
      sources: [{ kind: "user_text" }],
      translate: (key) => key,
      setStatus: vi.fn(),
      setDebug: vi.fn(),
    });
    expect(result).toMatchObject({
      kind: "succeeded",
      explicit: true,
      queries: ["the Red Sox are playing today, July 27, 2026"],
      receipts: [{ digest: "receipt-red-sox", invocationIndex: 1 }],
    });
    if (result.kind !== "succeeded") throw new Error("expected search success");
    expect(result.attachments[0]?.text).toContain("Native-Receipt: receipt-red-sox");
    expect(result.attachments[0]?.text).toContain("Invocation-Index: 1");
    expect(result.attachments[0]?.text).toContain("Result-Count: 3");
    expect(invokeMock).toHaveBeenCalledWith("sovereign_duckduckgo_search", {
      request: {
        query: "the Red Sox are playing today, July 27, 2026",
        originatingUtterance: utterance,
        maxResults: 5,
        sessionId: "session-red-sox",
        originTurnId: "turn-red-sox",
        originGenerationToken: "generation-red-sox",
      },
    });
  });

  it("returns receipt-backed Apple Spotlight evidence without opening a browser", async () => {
    const utterance =
      "Look online for Apple’s current macOS support page about Spotlight and give me the page title and link.";
    invokeMock.mockResolvedValueOnce({
      query: "Apple’s current macOS support page about Spotlight",
      engine: "duckduckgo_lite_static",
      resultCount: 1,
      degraded: false,
      contextJson: JSON.stringify({ pages: [{
        title: "Search for anything with Spotlight on Mac",
        url: "https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac",
        visibleText: "Use Spotlight to find apps, documents, email, and other items on your Mac.",
      }] }),
      retrievalElapsedMs: 20,
      domPageCount: 1,
      headlessFallbackCount: 0,
      receiptDigest: "a".repeat(64),
      invocationIndex: 1,
    });

    const result = await fetchLocalSearchAttachments({
      query: utterance,
      originTurnId: "turn-304",
      originGenerationToken: "generation-304",
      searchControlEnabled: false,
      targetSessionId: "session-304",
      sources: [{ kind: "user_text" }],
      translate: (key) => key,
      setStatus: vi.fn(),
      setDebug: vi.fn(),
    });

    expect(result).toMatchObject({
      kind: "succeeded",
      explicit: true,
      queries: ["Apple’s current macOS support page about Spotlight"],
      receipts: [{ digest: "a".repeat(64), invocationIndex: 1 }],
    });
    if (result.kind !== "succeeded") throw new Error("expected search success");
    expect(result.attachments[0]?.text).toContain("Search for anything with Spotlight on Mac");
    expect(result.attachments[0]?.text).toContain(
      "https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac",
    );
    expect(result.attachments[0]?.text).toContain(`Native-Receipt: ${"a".repeat(64)}`);
    expect(invokeMock).toHaveBeenLastCalledWith("sovereign_duckduckgo_search", {
      request: {
        query: "Apple’s current macOS support page about Spotlight",
        originatingUtterance: utterance,
        maxResults: 5,
        sessionId: "session-304",
        originTurnId: "turn-304",
        originGenerationToken: "generation-304",
      },
    });
    expect(invokeMock.mock.calls.every(([command]) => command === "sovereign_duckduckgo_search"))
      .toBe(true);
  });
});
