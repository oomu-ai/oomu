"use client";

type WorkflowAuthoringMetricEvent =
  | "compose_failed"
  | "compose_succeeded"
  | "edit_failed"
  | "edit_succeeded"
  | "run_failed"
  | "run_succeeded"
  | "save_failed"
  | "save_succeeded"
  | "template_loaded";

type WorkflowAuthoringMetrics = {
  composeFailed: number;
  composeSucceeded: number;
  editBeforeFirstRunTotal: number;
  editFailed: number;
  editSucceeded: number;
  firstRunCompleted: number;
  runFailed: number;
  runSucceeded: number;
  saveFailed: number;
  saveSucceeded: number;
  templateLoaded: number;
  updatedAtMs: number;
};

const STORAGE_KEY = "oomu.workflow.authoringMetrics.v1";

const DEFAULT_METRICS: WorkflowAuthoringMetrics = {
  composeFailed: 0,
  composeSucceeded: 0,
  editBeforeFirstRunTotal: 0,
  editFailed: 0,
  editSucceeded: 0,
  firstRunCompleted: 0,
  runFailed: 0,
  runSucceeded: 0,
  saveFailed: 0,
  saveSucceeded: 0,
  templateLoaded: 0,
  updatedAtMs: 0,
};

export function recordWorkflowAuthoringMetric(
  event: WorkflowAuthoringMetricEvent,
  options: { editCountBeforeFirstRun?: number; isFirstRun?: boolean } = {},
) {
  const storage = workflowMetricsStorage();
  if (!storage) {
    return;
  }

  const metrics = loadWorkflowAuthoringMetrics();
  const next: WorkflowAuthoringMetrics = {
    ...metrics,
    updatedAtMs: Date.now(),
  };

  switch (event) {
    case "compose_failed":
      next.composeFailed += 1;
      break;
    case "compose_succeeded":
      next.composeSucceeded += 1;
      break;
    case "edit_failed":
      next.editFailed += 1;
      break;
    case "edit_succeeded":
      next.editSucceeded += 1;
      break;
    case "run_failed":
      next.runFailed += 1;
      break;
    case "run_succeeded":
      next.runSucceeded += 1;
      if (options.isFirstRun) {
        next.firstRunCompleted += 1;
        next.editBeforeFirstRunTotal += options.editCountBeforeFirstRun ?? 0;
      }
      break;
    case "save_failed":
      next.saveFailed += 1;
      break;
    case "save_succeeded":
      next.saveSucceeded += 1;
      break;
    case "template_loaded":
      next.templateLoaded += 1;
      break;
  }

  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Metrics are intentionally best-effort and local-only.
  }
}

function loadWorkflowAuthoringMetrics(): WorkflowAuthoringMetrics {
  const storage = workflowMetricsStorage();
  if (!storage) {
    return DEFAULT_METRICS;
  }

  try {
    const parsed = JSON.parse(storage.getItem(STORAGE_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return DEFAULT_METRICS;
    }
    const record = parsed as Record<string, unknown>;
    return {
      composeFailed: numberValue(record.composeFailed),
      composeSucceeded: numberValue(record.composeSucceeded),
      editBeforeFirstRunTotal: numberValue(record.editBeforeFirstRunTotal),
      editFailed: numberValue(record.editFailed),
      editSucceeded: numberValue(record.editSucceeded),
      firstRunCompleted: numberValue(record.firstRunCompleted),
      runFailed: numberValue(record.runFailed),
      runSucceeded: numberValue(record.runSucceeded),
      saveFailed: numberValue(record.saveFailed),
      saveSucceeded: numberValue(record.saveSucceeded),
      templateLoaded: numberValue(record.templateLoaded),
      updatedAtMs: numberValue(record.updatedAtMs),
    };
  } catch {
    return DEFAULT_METRICS;
  }
}

function numberValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function workflowMetricsStorage() {
  if (typeof process !== "undefined" && process.env.NODE_ENV === "test") {
    return null;
  }

  if (typeof window === "undefined" || !window.localStorage) {
    return null;
  }

  const storage = window.localStorage;
  if (typeof storage.getItem !== "function" || typeof storage.setItem !== "function") {
    return null;
  }

  return storage;
}
