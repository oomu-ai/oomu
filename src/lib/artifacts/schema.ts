import { z } from "zod";

const sourceSchema = z.object({ sourceRef: z.string().min(1).max(256), evidenceRef: z.string().min(1).max(256), url: z.url().optional().nullable() }).strict();
const factual = { factual: z.boolean().default(false), sources: z.array(sourceSchema).max(32).default([]) };
const blockSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("paragraph"), text: z.string().min(1).max(20_000), style: z.enum(["body", "lead", "quote", "caption"]).default("body"), ...factual }).strict(),
  z.object({ type: z.literal("list"), ordered: z.boolean(), items: z.array(z.string().min(1).max(2_000)).min(1).max(100), ...factual }).strict(),
  z.object({ type: z.literal("table"), headers: z.array(z.string().max(2_000)).min(1).max(12), rows: z.array(z.array(z.string().max(2_000))).min(1).max(30), caption: z.string().max(500).default(""), ...factual }).strict(),
  z.object({ type: z.literal("callout"), label: z.string().min(1).max(120), text: z.string().min(1).max(4_000), ...factual }).strict(),
  z.object({ type: z.literal("citation"), label: z.string().min(1).max(500), url: z.url(), sourceRef: z.string().min(1).max(256), evidenceRef: z.string().min(1).max(256) }).strict(),
  z.object({ type: z.literal("page_break") }).strict(),
]);

export const artifactDocumentSchema = z.object({
  schemaVersion: z.literal(1),
  metadata: z.object({ title: z.string().min(1).max(240), subtitle: z.string().max(500).default(""), author: z.string().max(160).default(""), subject: z.string().max(500).default(""), keywords: z.array(z.string().max(80)).max(24).default([]), language: z.string().max(32).default("en") }).strict(),
  theme: z.object({ fontFamily: z.string().min(1).max(80), bodySizePt: z.number().min(8).max(18), titleSizePt: z.number().min(18).max(42), headingColor: z.string().regex(/^[0-9A-Fa-f]{6}$/), accentColor: z.string().regex(/^[0-9A-Fa-f]{6}$/), textColor: z.string().regex(/^[0-9A-Fa-f]{6}$/), backgroundColor: z.string().regex(/^[0-9A-Fa-f]{6}$/) }).strict(),
  page: z.object({ size: z.literal("letter"), orientation: z.literal("portrait"), marginTopIn: z.number().min(.5).max(2), marginRightIn: z.number().min(.5).max(2), marginBottomIn: z.number().min(.5).max(2), marginLeftIn: z.number().min(.5).max(2) }).strict(),
  header: z.string().max(500).nullable().optional(), footer: z.string().max(500).nullable().optional(),
  sections: z.array(z.object({ heading: z.string().min(1).max(240), pageBreakBefore: z.boolean().default(false), blocks: z.array(blockSchema).min(1).max(100) }).strict()).min(1).max(100),
}).strict().superRefine((document, context) => {
  for (const [sectionIndex, section] of document.sections.entries()) for (const [blockIndex, block] of section.blocks.entries()) {
    if ("factual" in block && block.factual && block.sources.length === 0) context.addIssue({ code: "custom", path: ["sections", sectionIndex, "blocks", blockIndex, "sources"], message: "Factual blocks require evidence." });
    if (block.type === "table" && block.rows.some((row) => row.length !== block.headers.length)) context.addIssue({ code: "custom", path: ["sections", sectionIndex, "blocks", blockIndex, "rows"], message: "Table rows must match headers." });
  }
});

export type ArtifactDocument = z.infer<typeof artifactDocumentSchema>;

export function createSimpleArtifactDocument(
  title: string,
  body: string,
  options: { language?: string; sources?: Array<{ sourceRef: string; evidenceRef: string; url?: string | null }> } = {},
): ArtifactDocument {
  const sources = (options.sources ?? []).slice(0, 32);
  return artifactDocumentSchema.parse({ schemaVersion: 1, metadata: { title, subtitle: "", author: "", subject: "", keywords: [], language: options.language ?? "en" }, theme: { fontFamily: "Arial", bodySizePt: 10.5, titleSizePt: 26, headingColor: "1F2937", accentColor: "2563EB", textColor: "111827", backgroundColor: "FFFFFF" }, page: { size: "letter", orientation: "portrait", marginTopIn: 1, marginRightIn: 1, marginBottomIn: 1, marginLeftIn: 1 }, header: null, footer: null, sections: [{ heading: title, pageBreakBefore: false, blocks: [{ type: "paragraph", text: body, style: "body", factual: sources.length > 0, sources }] }] });
}
