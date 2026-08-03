import { describe, expect, it } from "vitest";
import { directLocalFileReadPath } from "./directLocalFileRead";
import { parseLocalPathReferences } from "./localPathIntent";

describe("contextual file execution frontend boundary", () => {
  it("keeps a compound path-bearing request intact for normal intent and action routing", () => {
    const prompt = "Compare /Users/example/contracts/current.md with the proposal, then write a sprint into that folder.";
    const references = parseLocalPathReferences(prompt);

    expect(references.map((reference) => reference.normalizedText)).toEqual([
      "/Users/example/contracts/current.md",
    ]);
    expect(directLocalFileReadPath(
      prompt,
      references.map((reference) => reference.normalizedText),
    )).toBeNull();
    expect(prompt).toContain("then write a sprint into that folder");
  });

  it("does not invent a path for the deictic filename-clarification request", () => {
    const prompt = "Take your idea about the contract verification layer and write it into that folder. Use markdown format.";

    expect(parseLocalPathReferences(prompt)).toEqual([]);
  });
});
