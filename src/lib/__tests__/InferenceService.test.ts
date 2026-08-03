import { describe, expect, it } from "vitest";
import {
  InferenceStreamProcessor,
  InferenceTextAccumulator,
  sanitizeInferenceText,
} from "../InferenceService";

describe("InferenceStreamProcessor", () => {
  it("preserves raw whitespace chunks at token boundaries", () => {
    const processor = new InferenceStreamProcessor();

    expect(processor.push("Hello.")).toBe("Hello.");
    expect(processor.push(" ")).toBe(" ");
    expect(processor.push("World.")).toBe("World.");
    expect(processor.finish()).toBe("");
  });

  it("preserves leading protocol payload spaces across chunks", () => {
    const processor = new InferenceStreamProcessor();

    expect(processor.push('data: 0:"Whenever"\n')).toBe("Whenever");
    expect(processor.push('data: 0:" you"\n')).toBe(" you");
    expect(processor.push('data: 0:" go"\n')).toBe(" go");
    expect(processor.finish()).toBe("");
  });

  it("repairs missing word-boundary spaces across protocol chunks", () => {
    const processor = new InferenceStreamProcessor();

    expect(processor.push('data: 0:"Whenever"\n')).toBe("Whenever");
    expect(processor.push('data: 0:"you"\n')).toBe(" you");
    expect(processor.push('data: 0:"go"\n')).toBe(" go");
    expect(processor.finish()).toBe("");
  });

  it("repairs missing punctuation-boundary spaces across protocol chunks", () => {
    const streamed = ['data: 0:"Hello."', 'data: 0:"World."'].join("\n");

    expect(sanitizeInferenceText(streamed)).toBe("Hello. World.");
  });

  it("preserves punctuation and explicit blank-line protocol chunks", () => {
    const streamed = [
      'data: 0:"Hello."',
      'data: 0:" "',
      'data: 0:"World."',
      'data: 0:"\\n"',
      'data: 0:"\\nLine 2."',
    ].join("\n");

    expect(sanitizeInferenceText(streamed)).toBe("Hello. World.\n\nLine 2.");
  });

  it("passes raw multiline text through without boundary repair", () => {
    const processor = new InferenceStreamProcessor();

    expect(processor.push("Line 1.\n\nLine 2.")).toBe("Line 1.\n\nLine 2.");
    expect(processor.push("Still raw")).toBe("Still raw");
    expect(processor.finish()).toBe("");
  });

  it("strips Ministral channel and model control markup from final text", () => {
    expect(
      sanitizeInferenceText("The NASDAQ is down.</channel> <|model>Here are the corrected figures."),
    ).toBe("The NASDAQ is down. Here are the corrected figures.");
  });

  it("strips split Ministral control markup from live chunks", () => {
    const accumulator = new InferenceTextAccumulator();
    const chunks = ["The NASDAQ is down</chan", "nel> <|mod", "el>slightly."];

    expect(chunks.map((chunk) => accumulator.push(chunk)).join("")).toBe(
      "The NASDAQ is down slightly.",
    );
  });

  it("preserves ordinary angle-bracket text", () => {
    expect(sanitizeInferenceText("Use <section> for the wrapper.")).toBe(
      "Use <section> for the wrapper.",
    );
  });

  it("strips leaked text wrapper tags from assistant output", () => {
    expect(sanitizeInferenceText("<text>Visible answer.</text>")).toBe("Visible answer.");

    const accumulator = new InferenceTextAccumulator();
    const chunks = ["<te", "xt>Visible", " answer.</te", "xt>"];

    expect(chunks.map((chunk) => accumulator.push(chunk)).join("")).toBe(
      "Visible answer.",
    );
  });

  it("repairs live text boundaries matching the OOMU spacing regression", () => {
    const accumulator = new InferenceTextAccumulator();
    const chunks = [
      "I am writing this response to see if the",
      "spacing between words remains intact.",
      " That merging suggests",
      "there is still an edge case. ",
      "In",
      "tokenization, spaces can be attached to tokens. It might",
      "still get stripped and strips",
      "leading whitespace. We are almost",
      "there.",
    ];

    const text = chunks.map((chunk) => accumulator.push(chunk)).join("");

    expect(text).toContain("the spacing");
    expect(text).toContain("suggests there");
    expect(text).toContain("In tokenization");
    expect(text).toContain("might still");
    expect(text).toContain("strips leading");
    expect(text).toContain("almost there");
  });

  it("does not add spaces for common subword fragments", () => {
    const accumulator = new InferenceTextAccumulator();
    const chunks = ["Hel", "lo", " ", "com", "pletely", " ", "render", "ing", " ", "config", "uration"];

    expect(chunks.map((chunk) => accumulator.push(chunk)).join("")).toBe(
      "Hello completely rendering configuration",
    );
  });
});
