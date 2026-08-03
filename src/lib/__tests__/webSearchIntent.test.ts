import { describe, expect, it } from "vitest";
import {
  authorizeLocalWebSearch,
  exactVersionReleaseNotesSearchQuery,
  extractLocalWebSearchQuery,
  extractLocalWebSearchQueries,
  hasExplicitLocalWebSearchIntent,
  hasFreshnessLocalWebSearchIntent,
  hasLocalWebSearchIntent,
  hasPrivateLocalDataIntent,
  isLocalWebSearchAuthorized,
  isObjectiveBoundSearchContinuation,
} from "../webSearchIntent";
import localeMatrix from "./fixtures/search-intent/locale-matrix.json";
import authorityVectors from "../searchAuthorization/search-authority-vectors.json";

describe("Sprint 304 focused headless search", () => {
  it("binds the Sprint 304 Spotlight request to one focused headless query", () => {
    const prompt =
      "Look online for Apple’s current macOS support page about Spotlight and give me the page title and link.";

    expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(true);
    expect(extractLocalWebSearchQueries(prompt)).toEqual([
      "Apple’s current macOS support page about Spotlight",
    ]);
    expect(authorizeLocalWebSearch({
      utterance: prompt,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    })).toEqual({
      allowed: true,
      reason: "explicit_public_search",
      query: "Apple’s current macOS support page about Spotlight",
    });
  });

  it.each([
    ["Search online for Apple Spotlight support and return the title and URL", "Apple Spotlight support"],
    ["Look on the web for Apple Spotlight support and show me the link", "Apple Spotlight support"],
    ["Find online Apple Spotlight support and tell me the page title", "Apple Spotlight support"],
  ])("keeps delivery wording out of the headless query: %s", (prompt, query) => {
    expect(extractLocalWebSearchQuery(prompt)).toBe(query);
  });
});

describe("hasLocalWebSearchIntent", () => {
  it.each(authorityVectors)("matches the shared authority vector $id", (vector) => {
    expect(hasExplicitLocalWebSearchIntent(vector.utterance)).toBe(vector.explicit);
    expect(extractLocalWebSearchQuery(vector.utterance) || null).toBe(vector.query);
    expect(authorizeLocalWebSearch({
      utterance: vector.utterance,
      searchControlEnabled: false,
      sources: hasPrivateLocalDataIntent(vector.utterance)
        ? [{ kind: "private_local", source: "test", digest: "test" }]
        : [{ kind: "user_text" }],
    }).allowed).toBe(vector.authorized);
  });

  it("authorizes the exact Kimi/Fable incident wording regardless of ambient Search", () => {
    const utterance = authorityVectors[0].utterance;
    for (const searchControlEnabled of [false, true]) {
      expect(authorizeLocalWebSearch({
        utterance,
        searchControlEnabled,
        sources: [{ kind: "user_text" }],
      })).toEqual({
        allowed: true,
        reason: "explicit_public_search",
        query: "the new accusations that Kimi was trained from Fable",
      });
    }
  });

  it("extracts two objective-bound official release searches from one accepted turn", () => {
    const objective = "Go online and research the latest stable releases of Rust and Node.js from their official websites. Search each separately, compare their release dates, and cite both official sources.";
    const queries = extractLocalWebSearchQueries(objective);
    expect(queries).toEqual([
      "latest stable Rust release date official website",
      "latest stable Node.js release date official website",
    ]);
    expect(queries.every((query) => isObjectiveBoundSearchContinuation(objective, query))).toBe(true);
    expect(isObjectiveBoundSearchContinuation(objective, "private calendar events")).toBe(false);
  });

  it("authorizes the Scenario 07 conversational search and its version-bound continuation", () => {
    const objective = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";

    expect(authorizeLocalWebSearch({
      utterance: objective,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    })).toEqual({
      allowed: true,
      reason: "explicit_public_search",
      query: "the latest stable Rust release",
    });
    expect(isObjectiveBoundSearchContinuation(
      objective,
      "Rust 1.97.1 official release notes",
    )).toBe(true);
    expect(isObjectiveBoundSearchContinuation(
      objective,
      "Node.js 24.0.0 official release notes",
    )).toBe(false);
  });

  it("derives the exact-version release-notes lookup only from verified official context", () => {
    const objective = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";
    const context = JSON.stringify({
      pages: [
        {
          title: "The Rust Release Announcements",
          url: "https://blog.rust-lang.org/releases/",
          visibleText: "July 16 | Announcing Rust 1.97.1\nJuly 9 | Announcing Rust 1.97.0",
        },
      ],
    });
    expect(exactVersionReleaseNotesSearchQuery(
      objective,
      "the latest stable Rust release",
      context,
    )).toBe("Rust 1.97.1 official release notes");
    expect(exactVersionReleaseNotesSearchQuery(
      objective,
      "the latest stable Rust release",
      JSON.stringify({ pages: [{
        title: "Untrusted mirror",
        url: "https://example.com/rust-releases",
        visibleText: "Announcing Rust 9.99.0",
      }] }),
    )).toBe("");
  });

  it("does not treat freshness as explicit one-turn permission", () => {
    expect(hasLocalWebSearchIntent("Look up the newest Gemini release notes")).toBe(false);
    expect(hasLocalWebSearchIntent("Is the World Cup happening right now?")).toBe(false);
    expect(hasLocalWebSearchIntent("Who is the current president of France?")).toBe(false);
    expect(hasLocalWebSearchIntent("What is the Red Sox schedule today?")).toBe(false);
  });

  it("separates explicit search from freshness-only prompts", () => {
    expect(
      hasExplicitLocalWebSearchIntent("OOMU, use the internet to search for tour dates"),
    ).toBe(true);
    expect(
      hasFreshnessLocalWebSearchIntent("OOMU, use the internet to search for tour dates"),
    ).toBe(false);
    expect(hasExplicitLocalWebSearchIntent("What is the Red Sox score today?")).toBe(false);
    expect(hasFreshnessLocalWebSearchIntent("What is the Red Sox score today?")).toBe(true);
  });

  it("requires an explicit external-search phrase", () => {
    const explicitExternalRequests = [
      "Search online for market news",
      "Search the web for market news",
      "Search Google for market news",
      "Use the internet to search for market news",
      "Search DuckDuckGo for market news",
    ];
    for (const prompt of explicitExternalRequests) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(true);
      expect(isLocalWebSearchAuthorized(prompt, false)).toBe(true);
      expect(isLocalWebSearchAuthorized(prompt, true)).toBe(true);
    }
    expect(hasExplicitLocalWebSearchIntent("look up why the stock dropped")).toBe(false);
    expect(
      hasExplicitLocalWebSearchIntent("Can you search and tell me what drove the market lower?"),
    ).toBe(false);
    expect(
      hasExplicitLocalWebSearchIntent("Search the repository for Google OAuth code"),
    ).toBe(false);
    expect(
      hasExplicitLocalWebSearchIntent("Search this document for the word online"),
    ).toBe(false);
  });

  it("matches the native bounded official-source research directive", () => {
    const prompt =
      "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison.";
    expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(true);
    expect(extractLocalWebSearchQuery(prompt)).toBe(
      "scheduled/background agent capabilities in OpenClaw and Claude Cowork",
    );
    expect(authorizeLocalWebSearch({
      utterance: prompt,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    })).toMatchObject({
      allowed: true,
      reason: "explicit_public_search",
      query: "scheduled/background agent capabilities in OpenClaw and Claude Cowork",
    });
  });

  it("does not broaden official-source research to generic, private, or negated research", () => {
    for (const prompt of [
      "Research OpenClaw and Claude Cowork",
      "Research current official sources on my calendar",
      "Do not research current official web sources on OpenClaw",
      "Research current official sources on OpenClaw without using the internet",
    ]) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(false);
      expect(extractLocalWebSearchQuery(prompt)).toBe("");
    }
  });

  it("does not treat bare lookup wording as explicit web intent", () => {
    expect(hasExplicitLocalWebSearchIntent("Look up the implementation of class X")).toBe(false);
    expect(hasLocalWebSearchIntent("lookup this symbol in the repository")).toBe(false);
    expect(hasExplicitLocalWebSearchIntent("Look up the implementation of class X online")).toBe(
      true,
    );
  });
});

describe("web search privacy boundaries", () => {
  it("does not treat product names or retrospective questions as consent", () => {
    const nonConsentingPrompts = [
      "look up why the stock dropped",
      "check the market and tell me what changed",
      "Check Google Calendar for today's schedule",
      "Did you search online to verify that?",
      "Why did you search online for that?",
      "I did not ask you to search online",
      "I didn't ask you to search online",
      "How do I use the internet?",
      "How does web search work?",
      "Tell me how online search works",
      "The phrase search online appears in the UI",
      "Can we talk about how to search online?",
      "I searched online yesterday",
      "Please explain how to search online",
      "Do not search the web for this",
    ];
    for (const prompt of nonConsentingPrompts) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(false);
      expect(isLocalWebSearchAuthorized(prompt, true)).toBe(false);
    }
    for (const prompt of [
      "What is on the schedule today?",
      "Show me the latest release notes",
      "Who is the current president of France?",
    ]) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(false);
      expect(isLocalWebSearchAuthorized(prompt, false)).toBe(false);
      expect(isLocalWebSearchAuthorized(prompt, true)).toBe(true);
    }
  });

  it("does not treat local file analysis as web grounding", () => {
    expect(hasLocalWebSearchIntent("Review the attached file and summarize the risks")).toBe(false);
  });

  it("does not infer web intent from standalone market statements", () => {
    expect(hasExplicitLocalWebSearchIntent("What drove the market lower?")).toBe(false);
    expect(hasLocalWebSearchIntent("The stock dropped after earnings")).toBe(false);
  });

  it("does not mistake a Calendar read for explicit web-search consent", () => {
    const prompt = "Check my calendar and let me know what I have going on today";
    expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(false);
    expect(hasPrivateLocalDataIntent(prompt)).toBe(true);
    expect(isLocalWebSearchAuthorized(prompt, false)).toBe(false);
    expect(isLocalWebSearchAuthorized(prompt, true)).toBe(false);
  });

  it("never sends private app data as an external search query", () => {
    const explicitExternalPrivateSearchRequests = [
      "Search online for my appointments",
      "Search the web for my tasks",
      "Search Google for my camera roll",
      "Search DuckDuckGo for my music",
      "Search Google Calendar for today's events",
      "Search online for my Outlook Calendar",
      "Search the web for my Gmail inbox",
      "Search online for my iCloud Photos",
    ];
    const localPrivateSearchRequests = [
      "Search my email and see if there is anything from Maya that needs a reply",
      "Search my calendar and see whether I have anything tomorrow afternoon",
      "Search my reminders and see if anything for the launch is still open",
      "Search my Notes and find anything I saved about the compiler review",
      "Search my contacts and see if you can find Maya Allan",
      "Search my photo library and show me the newest process diagram",
      "Search my Apple Music library and list my recently added songs",
    ];
    for (const prompt of explicitExternalPrivateSearchRequests) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(true);
      expect(hasPrivateLocalDataIntent(prompt)).toBe(true);
      expect(isLocalWebSearchAuthorized(prompt, true)).toBe(false);
    }
    for (const prompt of localPrivateSearchRequests) {
      expect(hasExplicitLocalWebSearchIntent(prompt)).toBe(false);
      expect(hasPrivateLocalDataIntent(prompt)).toBe(true);
      expect(isLocalWebSearchAuthorized(prompt, true)).toBe(false);
    }
  });
});

describe("web search ambient authority", () => {
  it("honors an explicit public search for that turn when ambient Search is off", () => {
    const prompt = "Search the public web for the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration. Cite the exact source URL and access time.";
    const query = "the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration";
    expect(authorizeLocalWebSearch({ utterance: prompt, searchControlEnabled: false, sources: [{ kind: "user_text" }] })).toEqual({ allowed: true, reason: "explicit_public_search", query });
    expect(authorizeLocalWebSearch({ utterance: prompt, searchControlEnabled: true, sources: [{ kind: "user_text" }] })).toEqual({ allowed: true, reason: "explicit_public_search", query });
  });

  it("uses freshness-only wording only when ambient Search is on", () => {
    const prompt = "What is the Red Sox schedule today?";
    expect(authorizeLocalWebSearch({
      utterance: prompt,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    })).toEqual({
      allowed: true,
      reason: "ambient_freshness_search",
      query: prompt,
    });
    expect(authorizeLocalWebSearch({
      utterance: prompt,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(false);
  });
});

describe("extractLocalWebSearchQuery", () => {
  it("extracts a topic from explicit search commands", () => {
    expect(extractLocalWebSearchQuery("search the web for the show Outsourced.")).toBe(
      "the show Outsourced",
    );
    expect(extractLocalWebSearchQuery("Please search Google for \"Oomu privacy policy\"")).toBe(
      "Oomu privacy policy",
    );
    expect(extractLocalWebSearchQuery("Use the internet to find Rust 2.0 release notes")).toBe(
      "Rust 2.0 release notes",
    );
    expect(extractLocalWebSearchQuery("Look Blackpink tour dates up online")).toBe(
      "Blackpink tour dates",
    );
  });

  it("does not substitute an unrelated contextual topic into the network query", () => {
    expect(
      extractLocalWebSearchQuery("Search online for that", [
        { role: "user", content: "search the web for the show Outsourced." },
        { role: "assistant", content: "It looks like Outsourced was available on streaming services." },
      ]),
    ).toBe("");
  });

  it("does not return the literal followup when no topic is available", () => {
    expect(extractLocalWebSearchQuery("Did you search online to verify that?")).toBe("");
  });
});

describe("separate release query refinement", () => {
  it("preserves the date-less query shape when no release date was requested", () => {
    expect(extractLocalWebSearchQueries(
      "Go online and research the latest stable releases of Rust and Node.js from their official websites. Search each separately and cite both official sources.",
    )).toEqual([
      "latest stable release of Rust official website",
      "latest stable release of Node.js official website",
    ]);
  });
});

describe("provenance-first search authorization", () => {
  it.each(localeMatrix)("allows an explicit public topic in $locale for exactly that turn", ({ public: utterance }) => {
    expect(authorizeLocalWebSearch({
      utterance,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(true);
    expect(authorizeLocalWebSearch({
      utterance,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(true);
  });

  it.each(localeMatrix)("blocks private provenance in $locale regardless of wording", ({ private: utterance }) => {
    expect(authorizeLocalWebSearch({
      utterance,
      searchControlEnabled: true,
      sources: [{ kind: "private_local", source: "calendar", digest: "abc123" }],
    })).toEqual({ allowed: false, reason: "private_source" });
  });

  it.each(localeMatrix)("uses ambient Search for freshness but not ordinary wording in $locale", ({ ordinary, freshness }) => {
    expect(authorizeLocalWebSearch({
      utterance: ordinary,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(false);
    expect(authorizeLocalWebSearch({
      utterance: freshness,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    })).toMatchObject({ allowed: true, reason: "ambient_freshness_search" });
    expect(authorizeLocalWebSearch({
      utterance: freshness,
      searchControlEnabled: false,
      sources: [{ kind: "user_text" }],
    }).allowed).toBe(false);
  });

  it.each(localeMatrix)("fails closed for referential and mixed-language derived wording in $locale", ({ pronoun, mixed }) => {
    expect(authorizeLocalWebSearch({
      utterance: pronoun,
      searchControlEnabled: true,
      sources: [{ kind: "unknown_derived" }],
    }).reason).toBe("unknown_derived_source");
    expect(authorizeLocalWebSearch({
      utterance: mixed,
      searchControlEnabled: true,
      sources: [{ kind: "unknown_derived" }],
    }).reason).toBe("unknown_derived_source");
  });

  it.each(localeMatrix)("asks for a concrete topic instead of searching the $locale pronoun", ({ pronoun }) => {
    expect(authorizeLocalWebSearch({
      utterance: pronoun,
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    })).toMatchObject({ allowed: false, reason: "weak_query" });
  });

  it("blocks unknown derived data and referential-only queries", () => {
    expect(authorizeLocalWebSearch({
      utterance: "Search Google for lunar eclipses",
      searchControlEnabled: true,
      sources: [{ kind: "unknown_derived" }],
    }).reason).toBe("unknown_derived_source");
    expect(authorizeLocalWebSearch({
      utterance: "Search online for that",
      searchControlEnabled: true,
      sources: [{ kind: "user_text" }],
    }).reason).toBe("weak_query");
  });
});
