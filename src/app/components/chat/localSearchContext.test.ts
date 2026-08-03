import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  activePageAttachment,
  isDirectModNetworkRequest,
  localSearchAttachment,
  localSearchOutcomeStopsInference,
  fetchLocalSearchAttachments,
  shouldReadActivePage,
  shouldUseLocalWebSearch,
} from "./localSearchContext";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("provenance-first local search routing", () => {
  it("allows an explicit public query independently of ambient Search", () => {
    const utterance = "Search the public web for the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration. Cite the exact source URL and access time.";
    const query = "the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration";
    expect(shouldUseLocalWebSearch({
      utterance,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    })).toMatchObject({ allowed: true, reason: "explicit_public_search", query });
    expect(shouldUseLocalWebSearch({
      utterance,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    })).toMatchObject({ allowed: true, reason: "explicit_public_search", query });
  });

  it("uses an exact freshness query only while ambient Search is enabled", () => {
    const utterance = "What is new today?";
    expect(shouldUseLocalWebSearch({
      utterance,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    })).toEqual({
      allowed: true,
      reason: "ambient_freshness_search",
      query: utterance,
    });
    expect(shouldUseLocalWebSearch({
      utterance,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(false);
  });

  it("makes failed explicit grounding terminal while ordinary offline chat continues", () => {
    expect(localSearchOutcomeStopsInference({
      kind: "timed_out",
      explicit: true,
      errorCode: "search_retrieval_timeout",
    })).toBe(true);
    expect(localSearchOutcomeStopsInference({
      kind: "not_requested",
      explicit: false,
      authorization: { allowed: false, reason: "search_disabled" },
    })).toBe(false);
    expect(localSearchOutcomeStopsInference({
      kind: "unavailable",
      explicit: false,
      errorCode: "search_provider_unavailable",
    })).toBe(false);
  });

  it("fails closed for private and unclassified derived sources", () => {
    expect(shouldUseLocalWebSearch({
      utterance: "Search Google for my calendar tomorrow",
      searchControlEnabled: true,
      sources: [{ kind: "private_local", source: "calendar", digest: "digest" }],
    }).reason).toBe("private_source");
    expect(shouldUseLocalWebSearch({
      utterance: "Search online for that",
      searchControlEnabled: true,
      sources: [{ kind: "unknown_derived" }],
    }).reason).toBe("unknown_derived_source");
  });

  it("does not replace ordinary attached-file progress with a web-search warning", async () => {
    const setStatus = vi.fn();
    const outcome = await fetchLocalSearchAttachments({
      query: "Review the attached plan and include the Google OAuth link it already cites.",
      originTurnId: "turn-private-file",
      originGenerationToken: "generation-private-file",
      searchControlEnabled: true,
      targetSessionId: "session-private-file",
      sources: [{ kind: "unknown_derived" }],
      translate: (key) => key,
      setStatus,
      setDebug: vi.fn(),
    });

    expect(outcome).toMatchObject({
      kind: "not_requested",
      explicit: false,
      authorization: { reason: "unknown_derived_source" },
    });
    expect(setStatus).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("keeps the private-search warning for a real derived-data web request", async () => {
    const setStatus = vi.fn();
    const outcome = await fetchLocalSearchAttachments({
      query: "Search the web for what this private attachment says.",
      originTurnId: "turn-private-search",
      originGenerationToken: "generation-private-search",
      searchControlEnabled: true,
      targetSessionId: "session-private-search",
      sources: [{ kind: "unknown_derived" }],
      translate: (key) => key,
      setStatus,
      setDebug: vi.fn(),
    });

    expect(outcome).toMatchObject({
      kind: "blocked",
      explicit: true,
      errorCode: "search_not_authorized",
      authorization: { reason: "unknown_derived_source" },
    });
    expect(setStatus).toHaveBeenCalledWith("chat.status.private_search_blocked");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("requests a topic for a weak explicit directive without invoking search", async () => {
    const outcome = await fetchLocalSearchAttachments({
      query: "Search the web",
      originTurnId: "turn-needs-topic",
      originGenerationToken: "generation-needs-topic",
      searchControlEnabled: false,
      targetSessionId: "session-needs-topic",
      sources: [{ kind: "user_text" }],
      translate: (key) => key,
      setStatus: vi.fn(),
      setDebug: vi.fn(),
    });

    expect(outcome).toMatchObject({
      kind: "blocked",
      explicit: true,
      errorCode: "search_query_invalid",
      authorization: { allowed: false, reason: "weak_query" },
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe("provenance-first local search context", () => {
  it("performs the exact-version official release-notes lookup before inference", async () => {
    const objective = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";
    listenMock.mockResolvedValue(vi.fn());
    invokeMock
      .mockResolvedValueOnce({
        query: "the latest stable Rust release",
        engine: "duckduckgo_lite_static",
        resultCount: 2,
        degraded: false,
        contextJson: JSON.stringify({
          pages: [{
            title: "The Rust Release Announcements",
            url: "https://blog.rust-lang.org/releases/",
            visibleText: "July 16 | Announcing Rust 1.97.1\nJuly 9 | Announcing Rust 1.97.0",
          }],
        }),
        retrievalElapsedMs: 10,
        domPageCount: 1,
        headlessFallbackCount: 0,
      })
      .mockResolvedValueOnce({
        query: "Rust 1.97.1 official release notes",
        engine: "duckduckgo_lite_static",
        resultCount: 2,
        degraded: false,
        contextJson: JSON.stringify({
          pages: [{
            title: "Rust Release Notes",
            url: "https://doc.rust-lang.org/nightly/releases.html#version-1971-2026-07-16",
            visibleText: "Version 1.97.1 (2026-07-16)\nFix miscompilation in LLVM optimization.",
          }],
        }),
        retrievalElapsedMs: 11,
        domPageCount: 1,
        headlessFallbackCount: 0,
      });

    const result = await fetchLocalSearchAttachments({
      query: objective,
      originTurnId: "turn-scenario-07",
      originGenerationToken: "generation-scenario-07",
      searchControlEnabled: false,
      targetSessionId: "session-scenario-07",
      sources: [{ kind: "user_text" }],
      translate: (key) => key,
      setStatus: vi.fn(),
      setDebug: vi.fn(),
    });

    expect(result).toMatchObject({
      kind: "succeeded",
      queries: [
        "the latest stable Rust release",
        "Rust 1.97.1 official release notes",
      ],
    });
    if (result.kind !== "succeeded") throw new Error("expected successful search chain");
    expect(result.attachments.map((attachment) => attachment.name)).toEqual([
      "local_web_search.md",
      "local_web_search_2.md",
    ]);
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock.mock.calls[1]?.[1]).toMatchObject({
      request: { query: "Rust 1.97.1 official release notes" },
    });
  });

  it("tears down transient progress exactly once after a verified result", async () => {
    const unlisten = vi.fn();
    const statuses: string[] = [];
    listenMock.mockResolvedValue(unlisten);
    invokeMock.mockResolvedValue({
      query: "Rust stable release",
      engine: "duckduckgo_lite_static",
      resultCount: 1,
      degraded: false,
      contextJson: JSON.stringify({ results: [{ url: "https://www.rust-lang.org" }] }),
      retrievalElapsedMs: 12,
      domPageCount: 1,
      headlessFallbackCount: 0,
    });

    const result = await fetchLocalSearchAttachments({
      query: "Search the public web for the Rust stable release.",
      searchQuery: "Rust stable release",
      originTurnId: "turn-294",
      originGenerationToken: "generation-294",
      searchControlEnabled: true,
      targetSessionId: "session-294",
      sources: [{ kind: "user_text" }],
      translate: (key) => key,
      setStatus: (status) => statuses.push(status),
      setDebug: vi.fn(),
    });

    expect(result.kind).toBe("succeeded");
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(statuses.filter((status) => status === "chat.status.local_search_ready"))
      .toHaveLength(1);
    expect(statuses.at(-1)).toBe("chat.status.local_search_ready");
  });

  it("grants per-turn network authority only to an explicit mod with user-authored text", () => {
    expect(isDirectModNetworkRequest(
      "com.example.market_research",
      [{ kind: "user_text" }],
    )).toBe(true);
    expect(isDirectModNetworkRequest(undefined, [{ kind: "user_text" }])).toBe(false);
    expect(isDirectModNetworkRequest(
      "com.example.market_research",
      [{ kind: "unknown_derived" }],
    )).toBe(false);
    expect(isDirectModNetworkRequest(
      "com.example.market_research",
      [{ kind: "private_local", source: "calendar", digest: "digest" }],
    )).toBe(false);
  });

  it("labels sanitized DOM grounding without exposing a visible browser workflow", () => {
    const attachment = localSearchAttachment({
      query: "travel ROC to SIN",
      engine: "duckduckgo_lite_static",
      resultCount: 1,
      degraded: false,
      contextJson: JSON.stringify({
        results: [{ title: "Flight", url: "https://example.com", snippet: "From $1,120" }],
        pages: [{ visibleText: "From $1,120", extractionMethod: "headless_browser" }],
      }),
    });

    expect(attachment?.text).toContain("keyless public search plus sanitized DOM streaming");
    expect(attachment?.text).toContain("no visible browser panel");
    expect(attachment?.text).not.toContain("no browser automation");
  });

  it("routes an agnostic active-page summary without a new search", () => {
    expect(
      shouldReadActivePage(
        "Summarize the active webpage",
        true,
        true,
      ),
    ).toBe(true);
    expect(shouldReadActivePage("Search for browser news", true, true)).toBe(false);
    expect(shouldReadActivePage("Summarize the active webpage", false, true)).toBe(false);
  });

  it("packages active-page DOM as sanitized grounding context", () => {
    const attachment = activePageAttachment({
      contextJson: JSON.stringify({
        title: "Research",
        visibleText: "# Findings\n- Verified result",
        extractionMethod: "active_browser",
      }),
      retrievalElapsedMs: 18,
      usedHeadlessBrowser: false,
    });
    expect(attachment?.name).toBe("active_web_page.md");
    expect(attachment?.text).toContain("scripts, styles, frames, navigation");
    expect(attachment?.text).toContain("# Findings");
  });
});
