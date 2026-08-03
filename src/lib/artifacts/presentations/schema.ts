import { z } from "zod";

const id = z.string().trim().min(1).max(256).regex(/^[A-Za-z][A-Za-z0-9_-]*$/);
const text = z.string().max(32_767).refine((value) => !value.includes("\0"));
const color = z.string().regex(/^[0-9A-Fa-f]{6}$/);
const sha256 = z.string().regex(/^[0-9a-f]{64}$/);
const source = z.string().trim().min(1).max(1_024);

const presentationFrameSchema = z.object({
  x: z.number().int().nonnegative(),
  y: z.number().int().nonnegative(),
  width: z.number().int().positive(),
  height: z.number().int().positive(),
}).strict();

const textRunSchema = z.object({
  text,
  fontFamily: z.string().trim().min(1).max(128),
  fontSizePt: z.number().finite().min(4).max(200),
  bold: z.boolean().optional().default(false),
  italic: z.boolean().optional().default(false),
  color: color.optional().default("202124"),
}).strict();

const textParagraphSchema = z.object({
  runs: z.array(textRunSchema).max(256).optional().default([]),
  alignment: z.enum(["left", "center", "right"]).optional().default("left"),
  bullet: z.boolean().optional().default(false),
}).strict();

const textBlockSchema = z.object({
  paragraphs: z.array(textParagraphSchema).max(256).optional().default([]),
  verticalAlignment: z.enum(["top", "middle", "bottom"]).optional().default("top"),
}).strict();

const provenanceSchema = z.object({
  sourceRef: source,
  evidenceRef: source,
  note: z.string().trim().min(1).max(1_000).nullable().optional(),
}).strict();

const imageLicenseSchema = z.object({
  status: z.enum(["owned", "licensed", "public_domain", "unknown"]),
  sourceUrl: z.string().url().max(2_048).nullable().optional(),
  attribution: z.string().trim().min(1).max(1_000).nullable().optional(),
}).strict();

const elementContentSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("text_box"), text: textBlockSchema }).strict(),
  z.object({
    kind: z.literal("shape"),
    geometry: z.enum(["rectangle", "rounded_rectangle", "ellipse", "triangle", "line"]),
    fillColor: color,
    lineColor: color.nullable().optional(),
    text: textBlockSchema.nullable().optional(),
  }).strict(),
  z.object({
    kind: z.literal("image"),
    image: z.object({
      assetId: id,
      mediaType: z.enum(["png", "jpeg"]),
      bytesBase64: z.string().min(4).max(32 * 1024 * 1024).regex(/^[A-Za-z0-9+/]+={0,2}$/),
      widthPx: z.number().int().positive().max(32_768),
      heightPx: z.number().int().positive().max(32_768),
      altText: z.string().trim().min(1).max(2_000),
      license: imageLicenseSchema,
    }).strict(),
  }).strict(),
  z.object({
    kind: z.literal("table"),
    table: z.object({
      rows: z.array(z.array(textBlockSchema).min(1).max(50)).min(1).max(100),
      headerRow: z.boolean().optional().default(false),
    }).strict(),
  }).strict(),
  z.object({
    kind: z.literal("chart"),
    chart: z.object({
      chartType: z.enum(["column", "bar", "line", "pie"]),
      title: z.string().trim().min(1).max(512),
      categories: z.array(z.string().max(512)).min(1).max(1_000),
      series: z.array(z.object({
        name: z.string().trim().min(1).max(256),
        values: z.array(z.number().finite()).min(1).max(1_000),
      }).strict()).min(1).max(64),
    }).strict(),
  }).strict(),
]);

const presentationElementSchema = z.object({
  objectId: id,
  frame: presentationFrameSchema,
  content: elementContentSchema,
  provenance: z.array(provenanceSchema).max(128).optional().default([]),
}).strict();

const slideSchema = z.object({
  slideId: id,
  layoutId: id,
  title: z.string().trim().min(1).max(512).nullable().optional(),
  elements: z.array(presentationElementSchema).max(1_000).optional().default([]),
  notes: z.object({
    speakerNotes: z.string().max(32_767).optional().default(""),
    sourceRefs: z.array(source).max(256).optional().default([]),
  }).strict().optional().default({ speakerNotes: "", sourceRefs: [] }),
  animations: z.array(z.object({
    animationId: id,
    objectId: id,
    kind: z.string().trim().min(1).max(128),
  }).strict()).max(1_000).optional().default([]),
}).strict();

export const presentationIrSchema = z.object({
  schemaVersion: z.literal(1),
  title: z.string().trim().min(1).max(256),
  locale: z.string().trim().min(2).max(35),
  revision: z.number().int().positive(),
  aspectRatio: z.enum(["16:9", "4:3"]),
  theme: z.object({
    themeId: id,
    name: z.string().trim().min(1).max(128),
    colors: z.object({
      dark: color,
      light: color,
      accent1: color,
      accent2: color,
      accent3: color,
      accent4: color,
      hyperlink: color,
    }).strict(),
    fonts: z.object({
      heading: z.string().trim().min(1).max(128),
      body: z.string().trim().min(1).max(128),
    }).strict(),
  }).strict(),
  masters: z.array(z.object({
    masterId: id,
    name: z.string().trim().min(1).max(128),
    themeId: id,
    layoutIds: z.array(id).min(1).max(128),
  }).strict()).min(1).max(32),
  layouts: z.array(z.object({
    layoutId: id,
    masterId: id,
    name: z.string().trim().min(1).max(128),
    kind: z.enum(["title", "title_and_content", "section_header", "two_column", "blank", "custom"]),
    placeholders: z.array(z.object({
      placeholderId: id,
      kind: z.enum(["title", "subtitle", "body", "picture", "chart", "table", "footer", "slide_number"]),
      frame: presentationFrameSchema,
    }).strict()).max(64).optional().default([]),
  }).strict()).min(1).max(256),
  slides: z.array(slideSchema).min(1).max(1_000),
  citations: z.array(z.object({
    citationId: id,
    slideId: id,
    objectId: id.nullable().optional(),
    sourceRef: source,
    evidenceRef: source,
    label: z.string().trim().min(1).max(2_000),
    locator: z.string().trim().min(1).max(1_000).nullable().optional(),
  }).strict()).max(4_096).optional().default([]),
  policy: z.object({
    overflow: z.enum(["reject", "shrink_to_fit"]),
    missingFont: z.enum(["reject", "substitute_theme"]),
    imageLicense: z.enum(["require_known", "allow_unknown_with_warning"]),
    unsupportedAnimation: z.enum(["reject", "remove"]),
    minimumFontSizePt: z.number().finite().min(6).max(24),
    minimumImageDpi: z.number().int().min(72).max(600),
    allowedFonts: z.array(z.string().trim().min(1).max(128)).min(1).max(256).optional().default(["Arial", "Georgia"]),
  }).strict().optional().default({
    overflow: "reject",
    missingFont: "reject",
    imageLicense: "require_known",
    unsupportedAnimation: "reject",
    minimumFontSizePt: 10,
    minimumImageDpi: 144,
    allowedFonts: ["Arial", "Georgia"],
  }),
  template: z.object({
    templateId: id.nullable().optional(),
    name: z.string().trim().min(1).max(128).optional().default("OOMU native"),
    imported: z.boolean().optional().default(false),
    fingerprintSha256: z.union([z.literal(""), sha256]).optional().default(""),
  }).strict().optional().default({ name: "OOMU native", imported: false, fingerprintSha256: "" }),
}).strict().superRefine((deck, context) => {
  const size = deck.aspectRatio === "16:9"
    ? { width: 12_192_000, height: 6_858_000 }
    : { width: 9_144_000, height: 6_858_000 };
  const masters = uniqueIndex(deck.masters, (item) => item.masterId, context, ["masters"]);
  const layouts = uniqueIndex(deck.layouts, (item) => item.layoutId, context, ["layouts"]);
  const slides = uniqueIndex(deck.slides, (item) => item.slideId, context, ["slides"]);
  const objects = new Map<string, string>();

  deck.masters.forEach((master, index) => {
    if (master.themeId !== deck.theme.themeId) issue(context, ["masters", index, "themeId"], "Master must use the deck theme.");
    master.layoutIds.forEach((layoutId) => {
      const layout = layouts.get(layoutId);
      if (!layout || layout.masterId !== master.masterId) issue(context, ["masters", index, "layoutIds"], "Master contains an unknown layout.");
    });
  });
  deck.layouts.forEach((layout, index) => {
    if (!masters.has(layout.masterId)) issue(context, ["layouts", index, "masterId"], "Layout contains an unknown master.");
    layout.placeholders.forEach((placeholder, placeholderIndex) => frameWithin(placeholder.frame, size, context, ["layouts", index, "placeholders", placeholderIndex, "frame"]));
  });
  deck.slides.forEach((slide, slideIndex) => {
    if (!layouts.has(slide.layoutId)) issue(context, ["slides", slideIndex, "layoutId"], "Slide contains an unknown layout.");
    slide.elements.forEach((element, elementIndex) => {
      frameWithin(element.frame, size, context, ["slides", slideIndex, "elements", elementIndex, "frame"]);
      if (objects.has(element.objectId)) issue(context, ["slides", slideIndex, "elements", elementIndex, "objectId"], "Object identifiers must be unique across the deck.");
      objects.set(element.objectId, slide.slideId);
      if (element.content.kind === "chart") {
        const count = element.content.chart.categories.length;
        element.content.chart.series.forEach((series, seriesIndex) => {
          if (series.values.length !== count) issue(context, ["slides", slideIndex, "elements", elementIndex, "content", "chart", "series", seriesIndex, "values"], "Chart values must match its categories.");
        });
      }
    });
    slide.animations.forEach((animation, animationIndex) => {
      if (objects.get(animation.objectId) !== slide.slideId) issue(context, ["slides", slideIndex, "animations", animationIndex, "objectId"], "Animation target must be on the same slide.");
    });
  });
  deck.citations.forEach((citation, index) => {
    if (!slides.has(citation.slideId)) issue(context, ["citations", index, "slideId"], "Citation contains an unknown slide.");
    if (citation.objectId && objects.get(citation.objectId) !== citation.slideId) issue(context, ["citations", index, "objectId"], "Citation target must be on its slide.");
  });
});

export type PresentationIr = z.infer<typeof presentationIrSchema>;

type TaskPresentationCopy = {
  title: string;
  summary: string;
  locale: string;
  coverLabel: string;
  findingsTitle: string;
  sources: Array<{ sourceRef: string; evidenceRef: string }>;
};

export function createTaskSummaryPresentation(copy: TaskPresentationCopy): PresentationIr {
  const sources = copy.sources.slice(0, 64);
  const anchors = sources.map((item) => ({ ...item }));
  const sourceRefs = sources.map((item) => item.sourceRef);
  const body = summaryParagraphs(copy.summary);
  const deck = {
    schemaVersion: 1 as const,
    title: copy.title,
    locale: copy.locale,
    revision: 1,
    aspectRatio: "16:9" as const,
    theme: {
      themeId: "oomu_theme", name: "OOMU Light",
      colors: { dark: "171717", light: "FFFFFF", accent1: "007AFF", accent2: "34C759", accent3: "5856D6", accent4: "FF9500", hyperlink: "0066CC" },
      fonts: { heading: "Arial", body: "Arial" },
    },
    masters: [{ masterId: "oomu_master", name: "OOMU", themeId: "oomu_theme", layoutIds: ["cover_layout", "content_layout"] }],
    layouts: [
      { layoutId: "cover_layout", masterId: "oomu_master", name: "Cover", kind: "title" as const, placeholders: [] },
      { layoutId: "content_layout", masterId: "oomu_master", name: "Title and content", kind: "title_and_content" as const, placeholders: [] },
    ],
    slides: [{
      slideId: "cover", layoutId: "cover_layout", title: copy.title,
      elements: [
        { objectId: "cover_rule", frame: { x: 914_400, y: 1_142_000, width: 1_524_000, height: 48_000 }, content: { kind: "shape" as const, geometry: "rectangle" as const, fillColor: "007AFF", lineColor: null, text: null }, provenance: [] },
        { objectId: "cover_title", frame: { x: 914_400, y: 1_600_000, width: 9_600_000, height: 2_000_000 }, content: { kind: "text_box" as const, text: block(copy.title, 50, true) }, provenance: [] },
        { objectId: "cover_label", frame: { x: 914_400, y: 4_300_000, width: 8_000_000, height: 500_000 }, content: { kind: "text_box" as const, text: block(copy.coverLabel, 18, false, "585858") }, provenance: [] },
      ], notes: { speakerNotes: "", sourceRefs }, animations: [],
    }, {
      slideId: "findings", layoutId: "content_layout", title: copy.findingsTitle,
      elements: [
        { objectId: "findings_title", frame: { x: 914_400, y: 640_000, width: 10_000_000, height: 750_000 }, content: { kind: "text_box" as const, text: block(copy.findingsTitle, 36, true) }, provenance: [] },
        { objectId: "findings_body", frame: { x: 914_400, y: 1_650_000, width: 10_000_000, height: 4_250_000 }, content: { kind: "text_box" as const, text: { paragraphs: body.map((value) => paragraph(value, 20, body.length > 1)), verticalAlignment: "top" as const } }, provenance: anchors },
      ], notes: { speakerNotes: copy.summary.slice(0, 32_767), sourceRefs }, animations: [],
    }],
    citations: sources.map((item, index) => ({ citationId: `source_${index + 1}`, slideId: "findings", objectId: "findings_body", sourceRef: item.sourceRef, evidenceRef: item.evidenceRef, label: item.sourceRef, locator: null })),
    policy: { overflow: "shrink_to_fit" as const, missingFont: "reject" as const, imageLicense: "require_known" as const, unsupportedAnimation: "reject" as const, minimumFontSizePt: 16, minimumImageDpi: 144, allowedFonts: ["Arial"] },
    template: { templateId: null, name: "OOMU native", imported: false, fingerprintSha256: "" },
  };
  return presentationIrSchema.parse(deck);
}

function block(value: string, size: number, bold: boolean, runColor = "202124") {
  return { paragraphs: [paragraph(value, size, false, bold, runColor)], verticalAlignment: "top" as const };
}

function paragraph(value: string, size: number, bullet: boolean, bold = false, runColor = "202124") {
  return { runs: [{ text: value, fontFamily: "Arial", fontSizePt: size, bold, italic: false, color: runColor }], alignment: "left" as const, bullet };
}

function summaryParagraphs(value: string) {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return [""];
  return normalized.match(/.{1,220}(?:\s|$)/g)?.map((item) => item.trim()).filter(Boolean).slice(0, 6) ?? [normalized.slice(0, 220)];
}

function uniqueIndex<T>(items: T[], key: (item: T) => string, context: z.RefinementCtx, path: PropertyKey[]) {
  const values = new Map<string, T>();
  items.forEach((item, index) => {
    const value = key(item);
    if (values.has(value)) issue(context, [...path, index], "Identifiers must be unique.");
    values.set(value, item);
  });
  return values;
}

function frameWithin(frame: z.infer<typeof presentationFrameSchema>, size: { width: number; height: number }, context: z.RefinementCtx, path: PropertyKey[]) {
  if (frame.x + frame.width > size.width || frame.y + frame.height > size.height) issue(context, path, "Frame exceeds the slide canvas.");
}

function issue(context: z.RefinementCtx, path: PropertyKey[], message: string) {
  context.addIssue({ code: "custom", path, message });
}
