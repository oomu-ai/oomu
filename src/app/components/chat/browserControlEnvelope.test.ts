import { describe, expect, it } from "vitest";
import {
  BrowserControlEnvelopeAccumulator,
  projectBrowserControlEnvelope,
} from "./browserControlEnvelope";

const directive = [
  "<OomuSplitView>",
  "<mod_id>ai.eldris.mods.browser</mod_id>",
  "<action>NAVIGATE</action>",
  "<url>https://example.com</url>",
  "</OomuSplitView>",
].join("");

describe("reserved browser control envelopes", () => {
  it("withholds the envelope across every stream boundary", () => {
    const payload = `Before\n${directive}\nAfter`;
    for (let boundary = 1; boundary < payload.length; boundary += 1) {
      const accumulator = new BrowserControlEnvelopeAccumulator();
      const first = accumulator.push(payload.slice(0, boundary));
      const second = accumulator.push(payload.slice(boundary));
      const final = accumulator.finish();
      const visible = first.visibleDelta + second.visibleDelta + final.visibleDelta;
      expect(visible).toBe("Before\n\nAfter");
      expect(visible).not.toMatch(/OomuSplitView|mod_id|NAVIGATE|example\.com/);
      expect(final.directiveSnapshot).toBe(directive);
    }
  });

  it("keeps escaped and incomplete protocol material invisible and inert", () => {
    const escaped = directive
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
    expect(projectBrowserControlEnvelope(`Visible\n${escaped}`, true)).toEqual({
      visibleText: "Visible\n",
      directiveSnapshot: "",
      outcome: "malformed",
    });
    expect(projectBrowserControlEnvelope("Visible\n<OomuSplitView>hidden", true)).toEqual({
      visibleText: "Visible\n",
      directiveSnapshot: "",
      outcome: "malformed",
    });
  });

  it("buffers a possible reserved prefix until it is resolved", () => {
    const accumulator = new BrowserControlEnvelopeAccumulator();
    expect(accumulator.push("Answer\n<OomuS").visibleDelta).toBe("Answer\n");
    const completed = accumulator.push(directive.slice("<OomuS".length));
    expect(completed.visibleDelta).toBe("");
    expect(completed.directiveSnapshot).toBe(directive);
  });

  it("strips orphan closing tags without hiding adjacent prose", () => {
    expect(projectBrowserControlEnvelope("Cedar 14\n</OomuSplitView>", true)).toEqual({
      visibleText: "Cedar 14\n",
      directiveSnapshot: "",
      outcome: "malformed",
    });
  });
});

describe("typed Markdown control fences", () => {
  const typedFences = [
    "```oomu_search_request\n{\"query\":\"official Rust release notes\"}\n```",
    [
      "```json oomu_mcp_tool_call",
      "{\"serverName\":\"local_filesystem\",\"toolName\":\"read_file\"}",
      "```",
    ].join("\n"),
  ];

  it.each(typedFences)("withholds %s across every two-chunk boundary", (fence) => {
    const payload = `Before exactly.\n${fence}\nAfter exactly.`;
    for (let boundary = 1; boundary < payload.length; boundary += 1) {
      const accumulator = new BrowserControlEnvelopeAccumulator();
      const updates = [
        accumulator.push(payload.slice(0, boundary)),
        accumulator.push(payload.slice(boundary)),
        accumulator.finish(),
      ];
      const visible = updates.map((update) => update.visibleDelta).join("");
      expect(visible).toBe("Before exactly.\n\nAfter exactly.");
      expect(visible).not.toMatch(/oomu_(?:search_request|mcp_tool_call)|serverName|query/);
    }
  });

  it("keeps one-character streaming monotonic while preserving surrounding prose", () => {
    const payload = [
      "Opening prose.\n",
      typedFences[0],
      "\nMiddle prose.\n",
      typedFences[1],
      "\nClosing prose.",
    ].join("");
    const accumulator = new BrowserControlEnvelopeAccumulator();
    let accumulatedVisible = "";
    for (const token of payload) {
      const update = accumulator.push(token);
      expect(update.visibleText.startsWith(accumulatedVisible)).toBe(true);
      expect(update.visibleDelta).toBe(
        update.visibleText.slice(accumulatedVisible.length),
      );
      accumulatedVisible = update.visibleText;
      expect(accumulatedVisible).not.toMatch(
        /```|oomu_(?:search_request|mcp_tool_call)|official Rust|local_filesystem/,
      );
    }
    const final = accumulator.finish();
    accumulatedVisible += final.visibleDelta;
    expect(accumulatedVisible).toBe(
      "Opening prose.\n\nMiddle prose.\n\nClosing prose.",
    );
  });

  it("never exposes incomplete reserved fence prefixes", () => {
    const incomplete = [
      "```oomu_search_request\n{\"query\":\"bounded",
      "```json oomu_mcp_tool_ca",
    ];
    for (const marker of incomplete) {
      for (let length = 1; length <= marker.length; length += 1) {
        const projection = projectBrowserControlEnvelope(
          `Visible prose.\n${marker.slice(0, length)}`,
          false,
        );
        expect(projection.visibleText).toBe("Visible prose.\n");
        expect(projection.outcome).toBe("buffering");
      }
      expect(projectBrowserControlEnvelope(`Visible prose.\n${marker}`, true)).toEqual({
        visibleText: "Visible prose.\n",
        directiveSnapshot: "",
        outcome: "malformed",
      });
    }
  });

  it("leaves ordinary Markdown and split-view projection behavior intact", () => {
    const ordinary = "Use `inline code` and:\n```ts\nconst answer = 14;\n```\nDone.";
    expect(projectBrowserControlEnvelope(ordinary, true)).toEqual({
      visibleText: ordinary,
      directiveSnapshot: "",
      outcome: "clear",
    });
    const terminalFence = "```ts\nconst answer = 14;\n```";
    expect(projectBrowserControlEnvelope(terminalFence, true).visibleText).toBe(
      terminalFence,
    );

    const projection = projectBrowserControlEnvelope(
      `Before\n${typedFences[0]}\n${directive}\nAfter`,
      true,
    );
    expect(projection).toEqual({
      visibleText: "Before\n\n\nAfter",
      directiveSnapshot: directive,
      outcome: "complete",
    });
  });
});
