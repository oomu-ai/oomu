import { DYNAMIC_ROUTE_ID } from "./sessionRouting";

type SystemDiagnosticsSummary = {
  status: string;
  summary: string;
  durationMs: number;
  markdownReportPath?: string | null;
  markdownExported: boolean;
  system?: { environment?: { performance?: { warnings?: unknown[] } } };
  databaseFragmentation: { status: string }[];
  configurationHealth: { status: string }[];
  logs: { status: string }[];
};

export function isDynamicRouteId(value: string | null | undefined) {
  return value?.trim().toLowerCase() === DYNAMIC_ROUTE_ID;
}

export function sectionLineCount(content: string) {
  return content
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean).length;
}

export function isSystemDiagnosticsPrompt(prompt: string) {
  const normalized = prompt.trim();
  if (!normalized) return false;
  const noun = /\b(system diagnostics?|diagnostics?|diagnostic report|health check|health report)\b/i;
  const action = /\b(run|start|generate|export|write|check|scan)\b/i;
  return noun.test(normalized) && (action.test(normalized) || normalized.split(/\s+/).length <= 4);
}

export function systemDiagnosticsChatSummary(report: SystemDiagnosticsSummary) {
  const databaseAttention = report.databaseFragmentation.filter((check) =>
    ["attention", "unavailable"].includes(check.status),
  ).length;
  const configurationAttention = report.configurationHealth.filter((check) => check.status === "attention").length;
  const readableLogs = report.logs.filter((log) => log.status === "ok").length;
  const performanceWarnings = report.system?.environment?.performance?.warnings?.length ?? 0;
  return [
    "System diagnostics complete.",
    "",
    `Status: ${report.status}`,
    `Summary: ${report.summary}`,
    `Duration: ${report.durationMs} ms`,
    `Database attention flags: ${databaseAttention}`,
    `Configuration attention flags: ${configurationAttention}`,
    `Performance leak warnings: ${performanceWarnings}`,
    `Readable logs: ${readableLogs}/${report.logs.length}`,
    `Markdown report: ${report.markdownExported && report.markdownReportPath ? report.markdownReportPath : "not exported"}`,
  ].join("\n");
}
