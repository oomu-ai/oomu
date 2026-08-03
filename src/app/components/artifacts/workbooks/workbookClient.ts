import { invoke } from "@/lib/invoke";
import { workbookIrSchema, type WorkbookCell, type WorkbookIr } from "@/lib/artifacts/workbooks/schema";

const WORKBOOK_COMMANDS = {
  create: "create_workbook",
  inspectTemplate: "inspect_workbook_template",
  createFromTemplate: "create_workbook_from_template",
  list: "list_workbook_reviews",
  get: "get_workbook_review",
  preview: "get_workbook_preview",
  revise: "revise_workbook_range",
  exportRevision: "export_workbook_revision",
} as const;

type WorkbookCalculationStatus = "calculated" | "stale" | "failed" | "not_required" | "unavailable";
type WorkbookFormulaCell = { address: string; formula?: string; displayValue?: string; status: "calculated" | "stale" | "error" | "unavailable"; sourceRefs: string[] };
type WorkbookLineage = { range: string; sourceId: string; sourceLabel: string; observedAt?: string; freshness: "fresh" | "stale" | "unknown" };
type WorkbookWarning = { warningId: string; severity: "info" | "warning" | "blocked"; code: string; ranges: string[]; sheetId?: string };
export type WorkbookSheetReview = { sheetId: string; name: string; rowCount: number; columnCount: number; formulaCells: WorkbookFormulaCell[]; lineage: WorkbookLineage[]; warnings: WorkbookWarning[]; previewPageCount: number };
type WorkbookRevision = { revision: number; createdAt: string; instruction?: string; status: "verified" | "stale" | "failed" | "building" | "unavailable"; recoverable: boolean };
type WorkbookVersionReview = { revision: number; status: WorkbookRevision["status"]; calculation: { status: WorkbookCalculationStatus }; verification: WorkbookReview["verification"]; sheets: WorkbookSheetReview[]; exportable: boolean };
export type WorkbookReview = {
  schemaVersion: 1;
  artifactId: string;
  projectId: string;
  taskRunId: string;
  title: string;
  currentRevision: number;
  safePriorRevision?: number;
  calculation: { status: WorkbookCalculationStatus };
  verification: { status: "verified" | "stale" | "failed" | "unavailable"; structural: "passed" | "failed" | "not_run"; formula: "passed" | "failed" | "not_run"; visual: "passed" | "failed" | "not_run" };
  sheets: WorkbookSheetReview[];
  revisions: WorkbookRevision[];
  versionReviews: WorkbookVersionReview[];
  exportable: boolean;
};

type RawWorkbookSummary = { artifactId: string; safePriorRevision?: number };
type RawWorkbookWarning = { code: string; location: { sheetId?: string; range?: string; chartId?: string }; technicalDetail: string };
type RawWorkbookRevision = {
  revision: number;
  statusCode: "building" | "ready" | "needs_recalculation" | "check_required" | "failed";
  createdAtMs: number;
  completedAtMs?: number;
  sheets: Array<{ sheetId: string; name: string; previewAvailable: boolean }>;
  formulaCells: Array<{ sheetId: string; address: string; expression: string; displayValue: string; statusCode: "up_to_date" | "needs_recalculation" | "error" }>;
  lineage: Array<{ sheetId: string; address: string; sourceRef: string; evidenceRef: string }>;
  warnings: RawWorkbookWarning[];
  numbersStatusCode: "up_to_date" | "needs_recalculation" | "not_applicable";
  exportable: boolean;
  evidenceSummary: Array<{ code: string; passed: boolean; evidence: string }>;
  technicalEvidenceAvailable: boolean;
  recoverable: boolean;
  lastErrorCode?: string;
};
type RawWorkbookReview = {
  artifactId: string;
  projectId: string;
  taskId: string;
  taskRunId: string;
  title: string;
  currentRevision: number;
  selectedSheetId?: string;
  previewAvailable: boolean;
  safePriorRevision?: number;
  createdAtMs: number;
  updatedAtMs: number;
  revisions: RawWorkbookRevision[];
};

type WorkbookRevisionRequest = { artifactId: string; baseRevision: number; sheetId: string; targetRange?: string; instruction: string };
type WorkbookPreview = { artifactId: string; revision: number; sheetId: string; mimeType: string; dataUrl: string; width: number; height: number; sha256: string };
type CreateWorkbookFromTemplateRequest = {
  projectId: string;
  taskId: string;
  taskRunId: string;
  templateToken: string;
  title: string;
  locale: string;
  sheetName: string;
  targetRange?: string;
  instruction: string;
  replacementCells: WorkbookCell[];
};
type WorkbookTemplateInspection = {
  templateToken: string;
  taskRunId: string;
  sourceName: string;
  sourceSha256: string;
  sheets: Array<{
    sheetId: string;
    name: string;
    rowCount: number;
    columnCount: number;
    containsFormulas: boolean;
    visibility: "visible" | "hidden" | "very_hidden";
  }>;
  previewQualified: false;
  expiresAtMs: number;
};
type ExportWorkbookRevisionResult = {
  artifactId: string;
  revision: number;
  path: string;
  sha256: string;
  receiptId: string;
  accountingStatusCode: "recorded" | "recording_pending";
};

function normalizeReview(raw: RawWorkbookReview): WorkbookReview {
  const versionReviews = raw.revisions.map(normalizeVersion);
  const current = versionReviews.find((item) => item.revision === raw.currentRevision) ?? versionReviews[0];
  return {
    schemaVersion: 1,
    artifactId: raw.artifactId,
    projectId: raw.projectId,
    taskRunId: raw.taskRunId,
    title: raw.title,
    currentRevision: raw.currentRevision,
    safePriorRevision: raw.safePriorRevision,
    calculation: current?.calculation ?? { status: "unavailable" },
    verification: current?.verification ?? { status: "unavailable", structural: "not_run", formula: "not_run", visual: "not_run" },
    sheets: current?.sheets ?? [],
    revisions: raw.revisions.map((revision) => ({
      revision: revision.revision,
      createdAt: new Date(revision.createdAtMs).toISOString(),
      status: revision.statusCode === "building" ? "building" : revision.exportable && revision.statusCode === "ready" ? "verified" : revision.statusCode === "failed" ? "failed" : "stale",
      recoverable: revision.recoverable,
    })),
    versionReviews,
    exportable: Boolean(current?.exportable),
  };
}

function normalizeVersion(current: RawWorkbookRevision): WorkbookVersionReview {
  const calculation: WorkbookCalculationStatus = current.numbersStatusCode === "up_to_date"
    ? "calculated"
    : current.numbersStatusCode === "needs_recalculation"
      ? "stale"
      : current.numbersStatusCode === "not_applicable"
        ? "not_required"
        : "unavailable";
  const status = current.statusCode === "building"
    ? "building"
    : current.exportable && current.statusCode === "ready"
    ? "verified"
    : current.statusCode === "failed"
      ? "failed"
      : "stale";
  const sheets = current.sheets.map((sheet) => ({
    sheetId: sheet.sheetId,
    name: sheet.name,
    rowCount: 0,
    columnCount: 0,
    previewPageCount: sheet.previewAvailable ? 1 : 0,
    formulaCells: current.formulaCells.filter((cell) => cell.sheetId === sheet.sheetId).map((cell) => ({
      address: cell.address,
      formula: cell.expression,
      displayValue: cell.displayValue,
      status: cell.statusCode === "up_to_date" ? "calculated" as const : cell.statusCode === "needs_recalculation" ? "stale" as const : "error" as const,
      sourceRefs: [],
    })),
    lineage: current.lineage.filter((item) => item.sheetId === sheet.sheetId).map((item) => ({
      range: item.address,
      sourceId: item.sourceRef,
      sourceLabel: "",
      freshness: "unknown" as const,
    })),
    warnings: current.warnings.filter((warning) => !warning.location.sheetId || warning.location.sheetId === sheet.sheetId).map((warning, index) => ({
      warningId: `${warning.code}:${index}`,
      severity: warning.code === "critical_sheet_hidden" || warning.code === "package_relationship_invalid" ? "blocked" as const : "warning" as const,
      code: warning.code,
      ranges: warning.location.range ? [warning.location.range] : [],
      sheetId: warning.location.sheetId,
    })),
  }));
  return {
    revision: current.revision,
    status,
    calculation: { status: calculation },
    verification: {
      status: status === "building" ? "unavailable" : status,
      structural: status === "verified" ? "passed" : status === "failed" ? "failed" : "not_run",
      formula: calculation === "calculated" || calculation === "not_required" ? "passed" : "not_run",
      visual: status === "verified" ? "passed" : "not_run",
    },
    sheets,
    exportable: current.exportable,
  };
}

async function getReview(artifactId: string) {
  return normalizeReview(await invoke<RawWorkbookReview>(WORKBOOK_COMMANDS.get, { request: { artifactId } }));
}

export const workbookApi = {
  create: async (projectId: string, taskId: string, taskRunId: string, workbook: WorkbookIr) =>
    normalizeReview(await invoke<RawWorkbookReview>(WORKBOOK_COMMANDS.create, { request: { projectId, taskId, taskRunId, workbook: workbookIrSchema.parse(workbook) } })),
  inspectTemplate: (projectId: string, taskId: string, taskRunId: string) =>
    invoke<WorkbookTemplateInspection>(WORKBOOK_COMMANDS.inspectTemplate, {
      request: { projectId, taskId, taskRunId },
    }),
  createFromTemplate: (request: CreateWorkbookFromTemplateRequest) =>
    invoke<void>(WORKBOOK_COMMANDS.createFromTemplate, { request }),
  list: async (projectId?: string) => {
    const summaries = await invoke<RawWorkbookSummary[]>(WORKBOOK_COMMANDS.list, { request: { projectId: projectId || null } });
    return Promise.all(summaries.map((summary) => getReview(summary.artifactId)));
  },
  get: getReview,
  preview: (artifactId: string, revision: number, sheetId: string) =>
    invoke<WorkbookPreview>(WORKBOOK_COMMANDS.preview, { request: { artifactId, revision, sheetId } }),
  revise: async (request: WorkbookRevisionRequest) =>
    normalizeReview(await invoke<RawWorkbookReview>(WORKBOOK_COMMANDS.revise, { request: {
      artifactId: request.artifactId,
      baseRevision: request.baseRevision,
      sheetId: request.sheetId,
      ...(request.targetRange ? { targetRange: request.targetRange } : {}),
      instruction: request.instruction,
    } })),
  exportRevision: (artifactId: string, revision: number) =>
    invoke<ExportWorkbookRevisionResult>(WORKBOOK_COMMANDS.exportRevision, { request: { artifactId, revision } }),
};

export function canExportWorkbook(workbook: WorkbookReview) { return workbook.exportable; }
