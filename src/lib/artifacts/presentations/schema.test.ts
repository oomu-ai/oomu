import { describe, expect, it } from "vitest";
import { createTaskSummaryPresentation, presentationIrSchema } from "./schema";

describe("presentation IR", () => {
  it("creates a bounded editable task summary with inspectable sources", () => {
    const deck = createTaskSummaryPresentation({
      title: "Quarterly review",
      summary: "Revenue grew while support volume fell. The team can now choose the next investment.",
      locale: "en-US",
      coverLabel: "Project brief",
      findingsTitle: "What OOMU found",
      sources: [{ sourceRef: "task.completed", evidenceRef: "task-event:taskrun_55555555-5555-4555-8555-555555555555:4" }],
    });
    expect(deck.slides).toHaveLength(2);
    expect(deck.slides[1].elements[1].provenance).toHaveLength(1);
    expect(deck.citations[0]).toMatchObject({ slideId: "findings", objectId: "findings_body" });
  });

  it("rejects cross-slide citations, unknown layouts, and off-canvas content", () => {
    const valid = createTaskSummaryPresentation({
      title: "Review", summary: "A checked result.", locale: "en-US",
      coverLabel: "Project brief", findingsTitle: "What OOMU found", sources: [],
    });
    expect(() => presentationIrSchema.parse({ ...valid, slides: [{ ...valid.slides[0], layoutId: "missing" }] })).toThrow();
    expect(() => presentationIrSchema.parse({ ...valid, slides: valid.slides.map((slide, index) => index ? slide : { ...slide, elements: slide.elements.map((element, objectIndex) => objectIndex ? element : { ...element, frame: { ...element.frame, x: 12_000_000 } }) }) })).toThrow();
    expect(() => presentationIrSchema.parse({ ...valid, citations: [{ citationId: "bad", slideId: "cover", objectId: "findings_body", sourceRef: "source", evidenceRef: "evidence", label: "Source" }] })).toThrow();
  });

  it("accepts reusable object names across slides while keeping each slide exact", () => {
    const valid = createTaskSummaryPresentation({
      title: "Review", summary: "A checked result.", locale: "en-US",
      coverLabel: "Project brief", findingsTitle: "What OOMU found", sources: [],
    });
    const repeatedAcrossSlides = structuredClone(valid);
    repeatedAcrossSlides.slides[1].elements[0].objectId =
      repeatedAcrossSlides.slides[0].elements[0].objectId;
    expect(() => presentationIrSchema.parse(repeatedAcrossSlides)).not.toThrow();

    const duplicateOnOneSlide = structuredClone(valid);
    duplicateOnOneSlide.slides[0].elements[1].objectId =
      duplicateOnOneSlide.slides[0].elements[0].objectId;
    expect(() => presentationIrSchema.parse(duplicateOnOneSlide)).toThrow();
  });

  it("rejects hidden executable behavior, unlicensed images, and chart mismatch at the strict boundary", () => {
    const valid = createTaskSummaryPresentation({
      title: "Review", summary: "A checked result.", locale: "en-US",
      coverLabel: "Project brief", findingsTitle: "What OOMU found", sources: [],
    });
    expect(() => presentationIrSchema.parse({ ...valid, unexpected: true })).toThrow();
    expect(() => presentationIrSchema.parse({ ...valid, policy: { ...valid.policy, unsupportedAnimation: "allow" } })).toThrow();
    const chart = { objectId: "chart", frame: { x: 1, y: 1, width: 100, height: 100 }, provenance: [], content: { kind: "chart", chart: { chartType: "column", title: "Trend", categories: ["A", "B"], series: [{ name: "Value", values: [1] }] } } };
    expect(() => presentationIrSchema.parse({ ...valid, slides: [{ ...valid.slides[0], elements: [...valid.slides[0].elements, chart] }, valid.slides[1]] })).toThrow();
  });
});
