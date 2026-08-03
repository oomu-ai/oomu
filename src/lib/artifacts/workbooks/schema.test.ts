import { describe, expect, it } from "vitest";
import { createTaskSummaryWorkbook, workbookIrSchema } from "./schema";
import vectors from "./vectors.json";

const workbook = () => createTaskSummaryWorkbook({
  title: "Quarter summary", locale: "en-US", summary: "Revenue increased.", createdAtIso: "2026-07-11T16:00:00Z",
  labels: { sheet: "Summary", item: "Item", value: "Value", summary: "Summary", createdAt: "Created" },
  source: { sourceRef: "task-result", evidenceRef: "evidence-1" },
});

describe("shared Rust/TypeScript workbook vectors", () => {
  it("accepts the shared valid vector and rejects every shared invalid mutation", () => {
    expect(workbookIrSchema.safeParse(vectors.valid).success).toBe(true);
    for (const vector of vectors.invalid) {
      const candidate = structuredClone(vectors.valid) as unknown;
      for (const mutation of vector.mutations) setPath(candidate, mutation.path, mutation.value);
      expect(workbookIrSchema.safeParse(candidate).success, vector.case).toBe(false);
    }
  });
});

function setPath(target: unknown, path: (string | number)[], value: unknown): void {
  let cursor = target as Record<string | number, unknown>;
  for (const segment of path.slice(0, -1)) cursor = cursor[segment] as Record<string | number, unknown>;
  cursor[path.at(-1)!] = value;
}

describe("workbookIrSchema", () => {
  it("builds a strict localized task summary workbook", () => { expect(workbookIrSchema.parse(workbook()).worksheets[0].tables[0].name).toBe("TaskSummaryTable"); });
  it("rejects unknown fields and active formulas", () => {
    expect(() => workbookIrSchema.parse({ ...workbook(), unknown: true })).toThrow();
    const unsafe = workbook();
    unsafe.worksheets[0].cells[1].value = { kind: "formula", expression: "WEBSERVICE(A1)", cachedValue: null };
    unsafe.recalculation.status = "stale";
    expect(() => workbookIrSchema.parse(unsafe)).toThrow(/external or active/i);
  });
  it("rejects invalid Excel names and oversized inline validation lists", () => {
    const named = workbook(); named.namedRanges = [{ name: "A1", formula: "Summary!A1", comment: null }];
    expect(() => workbookIrSchema.parse(named)).toThrow(/Excel-safe/i);
    const list = workbook();
    list.worksheets[0].validations = [{ validationId: "choice", range: "A4", rule: { kind: "list", values: ["x".repeat(254)] }, allowBlank: false, prompt: null, error: null }];
    expect(() => workbookIrSchema.parse(list)).toThrow(/exceeds Excel's limit/i);
  });
});
