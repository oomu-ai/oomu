import { describe, expect, it } from "vitest";
import { canonicalAssistantDisplayText } from "./canonicalAssistantDisplay";

describe("canonical assistant display", () => {
  it("removes complete and orphan internal envelopes outside fenced examples", () => {
    const input = [
      "Visible result.",
      "<tool_call>{\"name\":\"read_file\"}</tool_call>",
      "<native_receipt>{\"verified\":true}</native_receipt>",
      "```xml",
      "<tool_call>{\"literal\":true}</tool_call>",
      "```",
      "Closing text.</tool_result>",
    ].join("\n");

    const output = canonicalAssistantDisplayText(input);
    expect(output).toContain("Visible result.");
    expect(output).toContain("Closing text.");
    expect(output).toContain("```xml\n<tool_call>{\"literal\":true}</tool_call>\n```");
    expect(output).not.toContain("read_file");
    expect(output).not.toContain("verified");
    expect(output).not.toContain("</tool_result>");
  });

  it("repairs concatenated headings without changing links, tables, quotes, or code", () => {
    const input = [
      "Summary### Recommendation",
      "Use the safe path.",
      "[Google](https://google.com/search?q=alpha#result)",
      "> Source quote",
      "| Item | State |",
      "| --- | --- |",
      "| One | Ready |",
      "```md",
      "literal### not a heading repair",
      "```",
    ].join("\n");

    const output = canonicalAssistantDisplayText(input);
    expect(output).toContain("Summary\n\n### Recommendation");
    expect(output).toContain("https://google.com/search?q=alpha#result");
    expect(output).toContain("> Source quote");
    expect(output).toContain("| Item | State |");
    expect(output).toContain("```md\nliteral### not a heading repair\n```");
  });

  it("repairs a fused OOMU artifact name without changing fenced sample text", () => {
    const input = [
      "I reviewed the attachment namedoomu_reliability_plan.md.",
      "```text",
      "namedoomu_reliability_plan.md",
      "```",
    ].join("\n");

    const output = canonicalAssistantDisplayText(input);
    expect(output).toContain("attachment named oomu_reliability_plan.md");
    expect(output).toContain("```text\nnamedoomu_reliability_plan.md\n```");
  });

  it("hides an unfinished internal envelope while a response is streaming", () => {
    const output = canonicalAssistantDisplayText(
      'Ready result.\n<tool_call>{"name":"read_file","arguments":',
    );

    expect(output).toBe("Ready result.");
    expect(output).not.toContain("read_file");
  });

  it("keeps an unfinished envelope literal inside a fenced example", () => {
    const input = "```xml\n<tool_call>{\\\"name\\\":\\\"example\\\"}\n```";
    expect(canonicalAssistantDisplayText(input)).toBe(input);
  });
});
