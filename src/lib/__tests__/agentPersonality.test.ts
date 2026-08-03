import { describe, expect, it } from "vitest";
import { buildAgentPersonalityPrompt } from "../agentPersonality";

describe("buildAgentPersonalityPrompt", () => {
  it("keeps agent identity while instructing first-person self-reference", () => {
    const prompt = buildAgentPersonalityPrompt({
      name: "OOMU",
      description: "Tracks finished work and calls out anything that needs attention.",
    });

    expect(prompt).toContain("Your active conversational name is OOMU.");
    expect(prompt).toContain('refer to yourself in first person as "I", "me", and "my"');
    expect(prompt).toContain("do not use OOMU as a third-person substitute for yourself");
  });
});
