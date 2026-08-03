import { z } from "zod";

const WORKBOOK_IR_SCHEMA_VERSION = 1 as const;
const A1 = /^\$?[A-Za-z]{1,3}\$?[1-9][0-9]{0,6}$/;
const RANGE = /^\$?[A-Za-z]{1,3}\$?[1-9][0-9]{0,6}(?::\$?[A-Za-z]{1,3}\$?[1-9][0-9]{0,6})?$/;
const RGB = /^[0-9A-Fa-f]{6}$/;
const SAFE_ID = /^[A-Za-z0-9_.-]{1,256}$/;
const SAFE_NAME = /^[A-Za-z_\\][A-Za-z0-9_.\\]{0,254}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const LOCALE = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/;
const DATE = /^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))?$/;

const formulaResultSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("number"), value: z.number().finite() }).strict(),
  z.object({ kind: z.literal("text"), value: z.string().max(32_767) }).strict(),
  z.object({ kind: z.literal("boolean"), value: z.boolean() }).strict(),
  z.object({ kind: z.literal("error"), code: z.enum(["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"]) }).strict(),
]);

const cellValueSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("blank") }).strict(),
  z.object({ kind: z.literal("text"), value: z.string().max(32_767) }).strict(),
  z.object({ kind: z.literal("number"), value: z.number().finite() }).strict(),
  z.object({ kind: z.literal("boolean"), value: z.boolean() }).strict(),
  z.object({ kind: z.literal("date"), iso: z.string().regex(DATE) }).strict(),
  z.object({ kind: z.literal("formula"), expression: z.string().min(1).max(8_192), cachedValue: formulaResultSchema.nullish() }).strict(),
]);

const provenanceSchema = z.object({
  sourceRef: z.string().min(1).max(256),
  evidenceRef: z.string().min(1).max(256),
  note: z.string().min(1).max(1_000).nullish(),
}).strict();

const cellSchema = z.object({
  address: z.string().regex(A1),
  value: cellValueSchema,
  formatId: z.string().min(1).max(256).nullish(),
  comment: z.object({ author: z.string().min(1).max(160), text: z.string().min(1).max(32_000) }).strict().nullish(),
  provenance: z.array(provenanceSchema).max(64).default([]),
}).strict();

const formatSchema = z.object({
  formatId: z.string().regex(SAFE_ID),
  font: z.object({ bold: z.boolean().default(false), italic: z.boolean().default(false), color: z.string().regex(RGB).nullish(), sizePt: z.number().finite().min(6).max(72).nullish() }).strict().default({ bold: false, italic: false }),
  fillColor: z.string().regex(RGB).nullish(),
  numberFormat: z.string().min(1).max(160).nullish(),
  alignment: z.enum(["general", "left", "center", "right"]).default("general"),
  wrapText: z.boolean().default(false),
}).strict();

const validationRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("list"), values: z.array(z.string().min(1).max(255)).min(1).max(100) }).strict(),
  z.object({ kind: z.literal("whole_number"), minimum: z.number().int(), maximum: z.number().int() }).strict(),
  z.object({ kind: z.literal("decimal"), minimum: z.number().finite(), maximum: z.number().finite() }).strict(),
  z.object({ kind: z.literal("date"), minimumIso: z.string().regex(DATE), maximumIso: z.string().regex(DATE) }).strict(),
  z.object({ kind: z.literal("custom_formula"), formula: z.string().min(1).max(8_192) }).strict(),
]);

const worksheetSchema = z.object({
  sheetId: z.string().regex(SAFE_ID),
  name: z.string().min(1).max(31),
  bounds: z.object({ rowCount: z.number().int().min(1).max(1_048_576), columnCount: z.number().int().min(1).max(16_384) }).strict(),
  visibility: z.enum(["visible", "hidden", "very_hidden"]).default("visible"),
  critical: z.boolean().default(false),
  cells: z.array(cellSchema).max(1_000_000).default([]),
  mergedRanges: z.array(z.string().regex(RANGE)).max(10_000).default([]),
  columnWidths: z.array(z.object({ column: z.string().regex(/^[A-Za-z]{1,3}$/), width: z.number().finite().min(1).max(255) }).strict()).max(16_384).default([]),
  tables: z.array(z.object({ tableId: z.string().regex(SAFE_ID), name: z.string().regex(SAFE_NAME), range: z.string().regex(RANGE), columns: z.array(z.string().min(1).max(255)).min(1).max(16_384), style: z.string().min(1).max(128).default("TableStyleMedium2") }).strict()).max(1_024).default([]),
  validations: z.array(z.object({ validationId: z.string().regex(SAFE_ID), range: z.string().regex(RANGE), rule: validationRuleSchema, allowBlank: z.boolean().default(false), prompt: z.string().min(1).max(255).nullish(), error: z.string().min(1).max(255).nullish() }).strict()).max(10_000).default([]),
  charts: z.array(z.object({ chartId: z.string().regex(SAFE_ID), kind: z.enum(["bar", "column", "line"]), title: z.string().min(1).max(240), categoryRange: z.string().min(1).max(512), series: z.array(z.object({ name: z.string().min(1).max(240), valueRange: z.string().min(1).max(512) }).strict()).min(1).max(32), anchor: z.object({ fromColumn: z.number().int().min(0), fromRow: z.number().int().min(0), toColumn: z.number().int().min(1), toRow: z.number().int().min(1) }).strict() }).strict()).max(256).default([]),
}).strict();

const recalculationSchema = z.object({
  status: z.enum(["not_required", "stale", "recalculated"]),
  engine: z.string().min(1).max(256).nullish(),
  engineVersion: z.string().min(1).max(128).nullish(),
  qualified: z.boolean().default(false),
  recalculatedAtMs: z.number().int().nullish(),
  inputDigest: z.string().regex(SHA256).nullish(),
}).strict();

const policySchema = z.object({
  macros: z.literal("forbid"), externalLinks: z.literal("forbid"),
  externalDataConnections: z.literal("forbid"), hiddenExecutableContent: z.literal("forbid"),
  hiddenCriticalSheets: z.literal("forbid"),
}).strict();

export const workbookIrSchema = z.object({
  schemaVersion: z.literal(WORKBOOK_IR_SCHEMA_VERSION),
  title: z.string().min(1).max(240), locale: z.string().regex(LOCALE),
  dateSystem: z.enum(["1900", "1904"]), revision: z.number().int().positive(),
  formats: z.array(formatSchema).max(10_000).default([]),
  worksheets: z.array(worksheetSchema).min(1).max(1_024),
  namedRanges: z.array(z.object({ name: z.string().regex(SAFE_NAME), formula: z.string().min(1).max(8_192), comment: z.string().min(1).max(255).nullish() }).strict()).max(10_000).default([]),
  recalculation: recalculationSchema,
  policy: policySchema,
}).strict().superRefine((workbook, context) => {
  const formatIds = uniqueSet(workbook.formats.map((format) => format.formatId));
  if (!formatIds) issue(context, ["formats"], "Format identifiers must be unique.");
  const sheetNames = workbook.worksheets.map((sheet) => sheet.name.toLocaleLowerCase("en-US"));
  if (!uniqueSet(sheetNames)) issue(context, ["worksheets"], "Worksheet names must be unique.");
  const tableNames = workbook.worksheets.flatMap((sheet) => sheet.tables.map((table) => table.name.toLocaleLowerCase("en-US")));
  const sheetsByName = new Map(workbook.worksheets.map((sheet) => [sheet.name.toLocaleLowerCase("en-US"), sheet]));
  if (!uniqueSet(tableNames)) issue(context, ["worksheets"], "Table names must be unique workbook-wide.");
  let formulaCount = 0;
  if (workbook.worksheets.reduce((count, sheet) => count + sheet.cells.length, 0) > 2_000_000 || stringByteCount(workbook) > 64 * 1024 * 1024) issue(context, ["worksheets"], "Workbook exceeds the bounded cell or text budget.");
  workbook.worksheets.forEach((sheet, sheetIndex) => {
    if (/[[\]:*?/\\]/.test(sheet.name) || sheet.name.startsWith("'") || sheet.name.endsWith("'")) issue(context, ["worksheets", sheetIndex, "name"], "Worksheet name is not Excel-safe.");
    if (sheet.critical && sheet.visibility !== "visible") issue(context, ["worksheets", sheetIndex, "visibility"], "Critical sheets must remain visible.");
    const addresses = new Set<string>();
    if (!uniqueSet(sheet.columnWidths.map((width) => width.column.toUpperCase()))) issue(context, ["worksheets", sheetIndex, "columnWidths"], "Column width declarations must be unique.");
    sheet.cells.forEach((cell, cellIndex) => {
      const normalized = cell.address.replaceAll("$", "").toUpperCase();
      if (addresses.has(normalized)) issue(context, ["worksheets", sheetIndex, "cells", cellIndex, "address"], "Cell addresses must be unique.");
      addresses.add(normalized);
      const address = parseCell(normalized);
      if (address && (address.row > sheet.bounds.rowCount || address.column > sheet.bounds.columnCount)) issue(context, ["worksheets", sheetIndex, "cells", cellIndex, "address"], "Cell exceeds declared bounds.");
      if (cell.formatId && !workbook.formats.some((format) => format.formatId === cell.formatId)) issue(context, ["worksheets", sheetIndex, "cells", cellIndex, "formatId"], "Cell format does not exist.");
      if (cell.value.kind === "formula") {
        formulaCount += 1;
        if (isUnsafeFormula(cell.value.expression)) issue(context, ["worksheets", sheetIndex, "cells", cellIndex, "value"], "Formula contains external or active content.");
        for (const reference of formulaReferences(cell.value.expression,sheet.name)) if (!referenceWithinBounds(reference,sheetsByName)) issue(context,["worksheets",sheetIndex,"cells",cellIndex,"value"],"Formula reference exceeds declared worksheet bounds.");
      }
    });
    sheet.tables.forEach((table, tableIndex) => {
      if (!uniqueSet(table.columns.map((column) => column.toLocaleLowerCase("en-US")))) issue(context, ["worksheets", sheetIndex, "tables", tableIndex, "columns"], "Table columns must be unique.");
      if (invalidExcelName(table.name)) issue(context, ["worksheets", sheetIndex, "tables", tableIndex, "name"], "Table name is not Excel-safe.");
      const range = parseRange(table.range);
      if (range) table.columns.forEach((column, offset) => {
        const address = `${columnName(range.start.column + offset)}${range.start.row}`;
        const header = sheet.cells.find((cell) => cell.address.replaceAll("$", "").toUpperCase() === address);
        if (header?.value.kind !== "text" || header.value.value !== column) issue(context, ["worksheets", sheetIndex, "tables", tableIndex, "columns", offset], "Table header cells must exactly match column names.");
      });
    });
    sheet.validations.forEach((validation, validationIndex) => {
      if (validation.rule.kind === "list") {
        const length = 2 + validation.rule.values.map((value) => value.replaceAll('"', '""').length).reduce((sum, value) => sum + value, 0) + validation.rule.values.length - 1;
        if (length > 255) issue(context, ["worksheets", sheetIndex, "validations", validationIndex, "rule"], "Inline validation list exceeds Excel's limit.");
      }
      if (validation.rule.kind === "date" && Date.parse(validation.rule.minimumIso) > Date.parse(validation.rule.maximumIso)) issue(context, ["worksheets", sheetIndex, "validations", validationIndex, "rule"], "Validation date minimum exceeds maximum.");
      if (validation.rule.kind === "custom_formula") { if(isUnsafeFormula(validation.rule.formula))issue(context,["worksheets",sheetIndex,"validations",validationIndex,"rule"],"Formula contains external or active content."); for (const reference of formulaReferences(validation.rule.formula,sheet.name)) if (!referenceWithinBounds(reference,sheetsByName)) issue(context,["worksheets",sheetIndex,"validations",validationIndex,"rule"],"Formula reference exceeds declared worksheet bounds."); }
    });
  });
  if (formulaCount > 0 && workbook.recalculation.status === "not_required") issue(context, ["recalculation", "status"], "Formula workbooks require recalculation status.");
  if (formulaCount === 0 && workbook.recalculation.status !== "not_required") issue(context, ["recalculation", "status"], "Formula-free workbooks do not require recalculation.");
  if (workbook.recalculation.status === "recalculated" && (!workbook.recalculation.qualified || !workbook.recalculation.engine || !workbook.recalculation.engineVersion || !workbook.recalculation.inputDigest || !workbook.recalculation.recalculatedAtMs)) issue(context, ["recalculation"], "Recalculation receipt is incomplete.");
  workbook.namedRanges.forEach((range, index) => { if (invalidExcelName(range.name)) issue(context, ["namedRanges", index, "name"], "Named range is not Excel-safe."); if(isUnsafeFormula(range.formula))issue(context,["namedRanges",index,"formula"],"Formula contains external or active content."); for(const reference of formulaReferences(range.formula,workbook.worksheets[0].name)) if(!referenceWithinBounds(reference,sheetsByName)) issue(context,["namedRanges",index,"formula"],"Formula reference exceeds declared worksheet bounds."); });
});

export type WorkbookIr = z.infer<typeof workbookIrSchema>;
export type WorkbookCell = z.infer<typeof cellSchema>;

function uniqueSet(values: string[]): boolean { return new Set(values).size === values.length; }
function issue(context: z.RefinementCtx, path: PropertyKey[], message: string): void { context.addIssue({ code: "custom", path, message }); }
function isUnsafeFormula(value: string): boolean { return /\[|\]|\||WEBSERVICE\s*\(|FILTERXML\s*\(|RTD\s*\(|DDE\s*\(|CALL\s*\(|EXEC\s*\(|REGISTER\.ID\s*\(|EVALUATE\s*\(|GET\.CELL\s*\(|GET\.WORKSPACE\s*\(|HYPERLINK\s*\(|_XLL\./i.test(value); }
function invalidExcelName(value: string): boolean { return /^(?:R|C|R(?:\[?-?\d+\]?)?C(?:\[?-?\d+\]?)?)$/i.test(value) || A1.test(value); }
function parseCell(value: string): { row: number; column: number } | null {
  const match = /^([A-Z]{1,3})([1-9][0-9]{0,6})$/.exec(value); if (!match) return null;
  let column = 0; for (const character of match[1]) column = column * 26 + character.charCodeAt(0) - 64;
  return { row: Number(match[2]), column };
}
function parseRange(value: string): { start: { row: number; column: number }; end: { row: number; column: number } } | null {
  const [start, end = start] = value.replaceAll("$", "").toUpperCase().split(":");
  const parsedStart = parseCell(start); const parsedEnd = parseCell(end); return parsedStart && parsedEnd ? { start: parsedStart, end: parsedEnd } : null;
}
interface FormulaReference { sheet:string; start:{row:number;column:number}; end:{row:number;column:number} }
function formulaReferences(formula:string,defaultSheet:string):FormulaReference[]{
  const references:FormulaReference[]=[];const pattern=/(?:(?:'((?:[^']|'')+)'|([A-Za-z_][A-Za-z0-9_.]*))!)?(\$?[A-Za-z]{1,3}\$?[0-9]{1,7})(?::(\$?[A-Za-z]{1,3}\$?[0-9]{1,7}))?/g;
  for(const match of formula.matchAll(pattern)){if(!match[1]&&!match[2]&&formula[match.index!+match[0].length]==="(")continue;const start=parseCell(match[3].replaceAll("$","").toUpperCase());const end=parseCell((match[4]??match[3]).replaceAll("$","").toUpperCase());if(start&&end)references.push({sheet:(match[1]?.replaceAll("''", "'")??match[2]??defaultSheet),start,end});}return references;
}
function referenceWithinBounds(reference:FormulaReference,sheets:Map<string,z.infer<typeof worksheetSchema>>):boolean{const sheet=sheets.get(reference.sheet.toLocaleLowerCase("en-US"));return Boolean(sheet&&reference.end.row<=sheet.bounds.rowCount&&reference.end.column<=sheet.bounds.columnCount);}
function columnName(column: number): string { let value = ""; for (let current = column; current > 0; current = Math.floor((current - 1) / 26)) value = String.fromCharCode(65 + ((current - 1) % 26)) + value; return value; }
function stringByteCount(value: unknown): number {
  if (typeof value === "string") return new TextEncoder().encode(value).byteLength;
  if (Array.isArray(value)) return value.reduce((sum, item) => sum + stringByteCount(item), 0);
  if (value && typeof value === "object") return Object.values(value).reduce((sum, item) => sum + stringByteCount(item), 0);
  return 0;
}

interface TaskSummaryWorkbookInput {
  title: string; locale: string; summary: string; createdAtIso: string;
  labels: { sheet: string; item: string; value: string; summary: string; createdAt: string };
  source?: { sourceRef: string; evidenceRef: string };
  sources?: Array<{ sourceRef: string; evidenceRef: string }>;
}

export function createTaskSummaryWorkbook(input: TaskSummaryWorkbookInput): WorkbookIr {
  const provenance = (input.sources ?? (input.source ? [input.source] : [])).slice(0, 64);
  return workbookIrSchema.parse({
    schemaVersion: 1, title: input.title, locale: input.locale, dateSystem: "1900", revision: 1,
    formats: [
      { formatId: "header", font: { bold: true, italic: false, color: "FFFFFF", sizePt: 11 }, fillColor: "2563EB", numberFormat: null, alignment: "left", wrapText: false },
      { formatId: "body_wrap", font: { bold: false, italic: false, color: "111827", sizePt: 11 }, fillColor: null, numberFormat: null, alignment: "left", wrapText: true },
    ],
    worksheets: [{
      sheetId: "task_summary", name: input.labels.sheet, bounds: { rowCount: 12, columnCount: 8 }, visibility: "visible", critical: true,
      cells: [
        { address: "A1", value: { kind: "text", value: input.labels.item }, formatId: "header", comment: null, provenance: [] },
        { address: "B1", value: { kind: "text", value: input.labels.value }, formatId: "header", comment: null, provenance: [] },
        { address: "A2", value: { kind: "text", value: input.labels.summary }, formatId: null, comment: null, provenance: [] },
        { address: "B2", value: { kind: "text", value: input.summary }, formatId: "body_wrap", comment: null, provenance },
        { address: "A3", value: { kind: "text", value: input.labels.createdAt }, formatId: null, comment: null, provenance: [] },
        { address: "B3", value: { kind: "date", iso: input.createdAtIso }, formatId: null, comment: null, provenance: [] },
      ], mergedRanges: [], columnWidths: [{ column: "A", width: 22 }, { column: "B", width: 70 }],
      tables: [{ tableId: "task_summary_table", name: "TaskSummaryTable", range: "A1:B3", columns: [input.labels.item, input.labels.value], style: "TableStyleMedium2" }], validations: [], charts: [],
    }],
    namedRanges: [], recalculation: { status: "not_required", engine: null, engineVersion: null, qualified: false, recalculatedAtMs: null, inputDigest: null },
    policy: { macros: "forbid", externalLinks: "forbid", externalDataConnections: "forbid", hiddenExecutableContent: "forbid", hiddenCriticalSheets: "forbid" },
  });
}
