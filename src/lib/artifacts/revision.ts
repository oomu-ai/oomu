import { artifactDocumentSchema, type ArtifactDocument } from "./schema";

export function applyRevisionInstruction(document: ArtifactDocument, rawInstruction: string): ArtifactDocument {
  const instruction = rawInstruction.trim();
  if (!instruction) throw new Error("Revision instruction is required.");
  const draft = structuredClone(document);
  const title = instruction.match(/^change title to\s+(.+)$/i);
  if (title) {
    draft.metadata.title = title[1].trim();
  } else {
    const replacement = instruction.match(/^replace\s+["“](.+?)["”]\s+with\s+["“](.+?)["”]$/i);
    if (replacement) {
      let changed = false;
      for (const section of draft.sections) for (const block of section.blocks) {
        if ((block.type === "paragraph" || block.type === "callout") && block.text.includes(replacement[1])) { block.text = block.text.replaceAll(replacement[1], replacement[2]); changed = true; }
      }
      if (!changed) throw new Error("Revision target was not found.");
    } else {
      const append = instruction.match(/^append section:\s*([^\n]+)\n([\s\S]+)$/i);
      if (!append) throw new Error("Use: Change title to..., Replace “...” with “...”, or Append section: Heading followed by body.");
      draft.sections.push({ heading: append[1].trim(), pageBreakBefore: false, blocks: [{ type: "paragraph", text: append[2].trim(), style: "body", factual: false, sources: [] }] });
    }
  }
  return artifactDocumentSchema.parse(draft);
}
