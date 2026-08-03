import { describe, expect, it } from "vitest";
import {
  contextualArtifactRouteDecision,
  isAgentOwnedMarkdownDestinationDeferred,
  isContextualMarkdownDestinationContinuation,
} from "./contextualArtifactContinuation";

describe("contextual Markdown destination continuation", () => {
  it("keeps an explicitly deferred destination conversational", () => {
    expect(isAgentOwnedMarkdownDestinationDeferred(
      "Write a test Markdown file and choose its filename. I will provide the destination next.",
    )).toBe(true);
    expect(contextualArtifactRouteDecision(
      "Write a Markdown file. I will provide the destination next.", [], "Thinking",
    )?.route).toBe("conversational_stream");
  });

  it("binds a path-only turn to the immediately preceding Markdown request", () => {
    expect(isContextualMarkdownDestinationContinuation(
      "/private/tmp/oomu-contextual-file-test",
      [
        {
          role: "user",
          content: "Write a test Markdown file with a short confirmation note. Choose its filename and content.",
        },
        { role: "system", content: "A destination is needed." },
      ],
    )).toBe(true);
    expect(contextualArtifactRouteDecision(
      "/private/tmp/oomu-contextual-file-test",
      [{ role: "user", content: "Write a Markdown report." }],
      "Planning",
    )?.route).toBe("agentic_planner");
  });

  it("does not revive an older file request after another user topic", () => {
    expect(isContextualMarkdownDestinationContinuation(
      "/private/tmp/oomu-contextual-file-test",
      [
        { role: "user", content: "Write a Markdown report." },
        { role: "user", content: "Instead, summarize the weather." },
      ],
    )).toBe(false);
  });

  it("does not treat ordinary slash text as a continuation without file intent", () => {
    expect(isContextualMarkdownDestinationContinuation(
      "/news",
      [{ role: "user", content: "Summarize the latest AI news." }],
    )).toBe(false);
  });
});
