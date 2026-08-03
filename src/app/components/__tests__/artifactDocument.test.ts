import { describe, expect, it } from "vitest";
import { createSimpleArtifactDocument } from "@/lib/artifacts/schema";
import { applyRevisionInstruction } from "@/lib/artifacts/revision";

describe("ArtifactDocument", () => {
  it("patches a validated document with bounded natural-language operations", () => {
    const original = createSimpleArtifactDocument("Initial", "Alpha evidence summary.");
    const replaced = applyRevisionInstruction(original, "Replace “Alpha” with “Verified”");
    expect(replaced.sections[0].blocks[0]).toMatchObject({ text: "Verified evidence summary." });
    const appended = applyRevisionInstruction(replaced, "Append section: Sources\nSource notes and evidence links.");
    expect(appended.sections.at(-1)?.heading).toBe("Sources");
  });

  it("rejects unstructured revision instructions instead of guessing", () => {
    const document = createSimpleArtifactDocument("Initial", "Body");
    expect(() => applyRevisionInstruction(document, "Make it better somehow")).toThrow();
  });

  it("preserves locale and bound factual source references", () => {
    const document = createSimpleArtifactDocument("Résumé", "Résultat vérifié", {
      language: "fr-FR",
      sources: [{ sourceRef: "connector.read_completed", evidenceRef: "task-event:taskrun_55555555-5555-4555-8555-555555555555:7" }],
    });
    expect(document.metadata.language).toBe("fr-FR");
    expect(document.sections[0].blocks[0]).toMatchObject({
      factual: true,
      sources: [{ sourceRef: "connector.read_completed", evidenceRef: "task-event:taskrun_55555555-5555-4555-8555-555555555555:7" }],
    });
  });
});
