"use client";

import { useEffect, useState } from "react";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { ActivityPane } from "./ActivityPane";
import { OperationsControl } from "./dashboard/OperationsControl";

type DatabaseFragmentationCheck = {
  name: string;
  status: string;
  fileBytes: number;
  walBytes: number;
  fragmentationRatio: number | null;
  detail: string;
};

type ConfigurationHealthCheck = {
  name: string;
  status: string;
  detail: string;
};

type LogSnapshot = {
  name: string;
  path?: string;
  status: string;
  sizeBytes: number;
};

type DisplaySnapshot = {
  index: number;
  name: string;
  frameX: number;
  frameY: number;
  frameWidth: number;
  frameHeight: number;
  isMain: boolean;
};

type IdeWindowSnapshot = {
  appName: string;
  title: string;
};

type NodeServerSnapshot = {
  processName: string;
  pid: number;
  port: number;
  listenAddress: string;
};

type GitWorkspaceSnapshot = {
  path: string;
  branch: string;
  dirty: boolean;
  changedFiles: number;
};

type CompilerProcessSnapshot = {
  pid: number;
  processName: string;
  command: string;
  residentMemoryBytes: number;
};

type MonitoredProcessMemorySnapshot = {
  pid: number;
  processName: string;
  command: string;
  residentMemoryBytes: number;
  category: string;
  recycleAllowed: boolean;
  restartStrategy?: string | null;
};

type PerformanceLeakWarning = {
  pid: number;
  processName: string;
  category: string;
  residentMemoryBytes: number;
  thresholdBytes: number;
  recycleAllowed: boolean;
  restartStrategy?: string | null;
  detail: string;
};

type AutonomicPerformanceSnapshot = {
  status: string;
  memoryWarningThresholdBytes: number;
  monitoredProcesses: MonitoredProcessMemorySnapshot[];
  warnings: PerformanceLeakWarning[];
  recycleAllowlist: string[];
  probeStatus: { name: string; status: string; detail: string }[];
};

type OperatingEnvironmentSnapshot = {
  displays: DisplaySnapshot[];
  ideWindows: IdeWindowSnapshot[];
  nodeServers: NodeServerSnapshot[];
  gitWorkspaces: GitWorkspaceSnapshot[];
  compilerProcesses: CompilerProcessSnapshot[];
  probeStatus: { name: string; status: string; detail: string }[];
  performance?: AutonomicPerformanceSnapshot;
};

type SystemSnapshot = {
  environment?: OperatingEnvironmentSnapshot;
};

type SystemDiagnosticsReport = {
  status: string;
  summary: string;
  durationMs: number;
  completedAtMs: number;
  markdownReportPath: string | null;
  markdownExported: boolean;
  markdownExportStatus: string;
  system?: SystemSnapshot;
  databaseFragmentation: DatabaseFragmentationCheck[];
  configurationHealth: ConfigurationHealthCheck[];
  logs: LogSnapshot[];
};

type DiagnosticsState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "done"; report: SystemDiagnosticsReport }
  | { phase: "error"; message: string };

type CompileLogEvent = {
  target: string;
  phase: string;
  stream: string;
  line: string;
  elapsedMs: number;
  exitCode?: number | null;
};

type CompileRefreshEvent = {
  target: string;
  reason: string;
};

const MAX_COMPILE_LOG_EVENTS = 200;

export function diagnosticLogKey(log: Pick<LogSnapshot, "name" | "path" | "sizeBytes">, index: number) {
  return log.path ?? `${log.name}-${log.sizeBytes}-${index}`;
}

export function compileLogKey(event: Pick<CompileLogEvent, "target" | "phase" | "stream" | "elapsedMs">, index: number) {
  return `${event.target}-${event.phase}-${event.stream}-${event.elapsedMs}-${index}`;
}

function describeError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

function formatBytes(bytes: number) {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(2)} KB`;
  return `${bytes} B`;
}

function formatPercent(value: number | null) {
  return value === null ? "n/a" : `${(value * 100).toFixed(1)}%`;
}

function formatDisplayFrame(display: DisplaySnapshot) {
  return `${Math.round(display.frameWidth)}x${Math.round(display.frameHeight)} @ ${Math.round(display.frameX)}, ${Math.round(display.frameY)}`;
}

function statusClass(status: string) {
  if (status === "passed" || status === "ok" || status === "written") {
    return "text-[var(--success)]";
  }
  if (status === "missing" || status === "skipped") {
    return "text-[var(--foreground-muted)]";
  }
  return "text-[var(--warning)]";
}

function compileStreamClass(stream: string) {
  if (stream === "stderr") return "text-[var(--warning)]";
  if (stream === "system") return "text-[var(--foreground-subtle)]";
  return "text-[var(--foreground-muted)]";
}

export function DeveloperPanel() {
  if (!isDeveloperBuild) {
    return null;
  }

  return (
    <section className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <header className="flex flex-col gap-2">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-[var(--foreground-subtle)]">
          Developer
        </p>
        <h1 className="text-2xl font-semibold tracking-tight text-[var(--foreground)]">
          Diagnostics
        </h1>
      </header>

      <SystemDiagnosticsPanel />

      <OperationsControl />

      <CompileLogPanel />

      {isDeveloperBuild ? (
        <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
          <div className="mb-4">
            <h2 className="text-sm font-semibold text-[var(--foreground)]">Activity Console</h2>
          </div>
          <ActivityPane />
        </section>
      ) : null}
    </section>
  );
}

function CompileLogPanel() {
  const [events, setEvents] = useState<CompileLogEvent[]>([]);
  const [refreshEvent, setRefreshEvent] = useState<CompileRefreshEvent | null>(null);
  const latest = events.length ? events[events.length - 1] : null;

  useEffect(() => {
    if (!isTauriRuntime) return;

    let disposed = false;
    const unlisteners: Array<() => void> = [];
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenLog = await listen<CompileLogEvent>("codebase-compile-log", (event) => {
          if (disposed) return;
          setEvents((current) => [...current, event.payload].slice(-MAX_COMPILE_LOG_EVENTS));
        });
        if (disposed) {
          unlistenLog();
          return;
        }
        unlisteners.push(unlistenLog);

        const unlistenRefresh = await listen<CompileRefreshEvent>("codebase-compile-refresh", (event) => {
          if (disposed) return;
          setRefreshEvent(event.payload);
          window.setTimeout(() => window.location.reload(), 250);
        });
        if (disposed) {
          unlistenRefresh();
          return;
        }
        unlisteners.push(unlistenRefresh);
      } catch (error) {
        setEvents((current) => [
          ...current,
          {
            target: "frontend",
            phase: "listener",
            stream: "stderr",
            line: describeError(error),
            elapsedMs: 0,
            exitCode: null,
          },
        ].slice(-MAX_COMPILE_LOG_EVENTS));
      }
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h2 className="text-sm font-semibold text-[var(--foreground)]">Compilation</h2>
          {latest ? (
            <p className="mt-1.5 text-sm leading-6 text-[var(--foreground-muted)]">
              {latest.target} {latest.phase} - {latest.stream}
            </p>
          ) : null}
        </div>
        <span className="shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-2.5 py-1 text-xs font-medium text-[var(--foreground-muted)]">
          {latest ? `${Math.round(latest.elapsedMs / 1000)}s` : "idle"}
        </span>
      </div>

      <div className="mt-4 max-h-64 overflow-y-auto rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--fill)] p-3 font-mono text-xs leading-5">
        {events.length === 0 ? (
          <p className="text-[var(--foreground-subtle)]">Waiting for compile events.</p>
        ) : (
          events.map((event, index) => (
            <p className="whitespace-pre-wrap break-words" key={compileLogKey(event, index)}>
              <span className="text-[var(--foreground-subtle)]">
                [{event.target}/{event.phase}]
              </span>{" "}
              <span className={compileStreamClass(event.stream)}>{event.stream}</span>{" "}
              <span className="text-[var(--foreground)]">{event.line}</span>
            </p>
          ))
        )}
      </div>

      {refreshEvent ? (
        <p className="mt-3 text-xs text-[var(--foreground-muted)]" aria-live="polite">
          Refreshing {refreshEvent.target} view.
        </p>
      ) : null}
    </section>
  );
}

function SystemDiagnosticsPanel() {
  const [state, setState] = useState<DiagnosticsState>({ phase: "idle" });

  async function runDiagnostics() {
    setState({ phase: "running" });
    try {
      const report = await invoke<SystemDiagnosticsReport>("run_system_diagnostics", {
        request: {
          exportMarkdown: true,
          includeMemoryAudit: true,
          includePreAlphaAudit: true,
          preAlphaRuns: 1,
        },
      });
      setState({ phase: "done", report });
    } catch (error) {
      setState({ phase: "error", message: describeError(error) });
    }
  }

  const isRunning = state.phase === "running";

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h2 className="text-sm font-semibold text-[var(--foreground)]">System Suite</h2>
          {state.phase === "done" ? (
            <p className="mt-1.5 text-sm leading-6 text-[var(--foreground-muted)]">
              {state.report.summary}
            </p>
          ) : null}
          {state.phase === "error" ? (
            <p className="mt-1.5 text-sm leading-6 text-[var(--destructive)]">
              {state.message}
            </p>
          ) : null}
        </div>
        <button
          className="shrink-0 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:opacity-50"
          disabled={isRunning}
          onClick={() => void runDiagnostics()}
          type="button"
        >
          {isRunning ? "Running..." : "Run Suite"}
        </button>
      </div>

      {state.phase === "running" ? (
        <p className="mt-4 text-xs text-[var(--foreground-muted)]" aria-live="polite">
          Collecting diagnostics...
        </p>
      ) : null}

      {state.phase === "done" ? <DiagnosticsReportSummary report={state.report} /> : null}

      {!isTauriRuntime ? (
        <p className="mt-4 text-xs leading-5 text-[var(--foreground-subtle)]">
          System diagnostics require the native desktop runtime.
        </p>
      ) : null}
    </section>
  );
}

function DiagnosticsReportSummary({ report }: { report: SystemDiagnosticsReport }) {
  return (
    <div className="mt-5 flex flex-col gap-5">
      <div className="grid gap-3 sm:grid-cols-3">
        <Metric label="Status" value={report.status} valueClassName={statusClass(report.status)} />
        <Metric label="Duration" value={`${report.durationMs} ms`} />
        <Metric label="Markdown" value={report.markdownExportStatus} valueClassName={statusClass(report.markdownExportStatus)} />
      </div>

      {report.markdownReportPath ? (
        <div className="rounded-[var(--radius-base)] border border-[var(--border-soft)] px-3 py-2">
          <p className="text-xs font-medium text-[var(--foreground-muted)]">Report Path</p>
          <p className="mt-1 break-all font-mono text-xs text-[var(--foreground)]">
            {report.markdownReportPath}
          </p>
        </div>
      ) : null}

      {report.system?.environment ? (
        <EnvironmentSummary environment={report.system.environment} />
      ) : null}

      <div>
        <h3 className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
          Databases
        </h3>
        <div className="mt-2 divide-y divide-[var(--border-soft)] rounded-[var(--radius-base)] border border-[var(--border-soft)]">
          {report.databaseFragmentation.map((check) => (
            <div className="grid gap-2 px-3 py-2.5 text-xs sm:grid-cols-[1fr_auto_auto]" key={check.name}>
              <span className="font-medium text-[var(--foreground)]">{check.name}</span>
              <span className={statusClass(check.status)}>{check.status}</span>
              <span className="text-[var(--foreground-muted)]">
                {formatPercent(check.fragmentationRatio)} free, {formatBytes(check.fileBytes + check.walBytes)}
              </span>
            </div>
          ))}
        </div>
      </div>

      <div>
        <h3 className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
          Configuration
        </h3>
        <div className="mt-2 grid gap-2 sm:grid-cols-2">
          {report.configurationHealth.map((check) => (
            <div className="rounded-[var(--radius-base)] border border-[var(--border-soft)] px-3 py-2.5" key={check.name}>
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs font-medium text-[var(--foreground)]">{check.name}</p>
                <span className={`text-xs ${statusClass(check.status)}`}>{check.status}</span>
              </div>
              <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">{check.detail}</p>
            </div>
          ))}
        </div>
      </div>

      <div>
        <h3 className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
          Logs
        </h3>
        <div className="mt-2 flex flex-wrap gap-2">
          {report.logs.map((log, index) => (
            <span
              className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-2.5 py-1 text-xs text-[var(--foreground-muted)]"
              key={diagnosticLogKey(log, index)}
            >
              {log.name}: <span className={statusClass(log.status)}>{log.status}</span>
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

function EnvironmentSummary({ environment }: { environment: OperatingEnvironmentSnapshot }) {
  const firstGitWorkspace = environment.gitWorkspaces[0];
  const firstNodeServer = environment.nodeServers[0];
  const firstCompiler = environment.compilerProcesses[0];
  const performance = environment.performance;
  const leakWarnings = performance?.warnings ?? [];

  return (
    <div>
      <h3 className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
        Operating Environment
      </h3>
      <div className="mt-2 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Metric label="Displays" value={String(environment.displays.length)} />
        <Metric label="Node Ports" value={environment.nodeServers.map((server) => String(server.port)).join(", ") || "none"} />
        <Metric
          label="Git Branch"
          value={firstGitWorkspace ? firstGitWorkspace.branch : "none"}
          valueClassName={firstGitWorkspace?.dirty ? "text-[var(--warning)]" : undefined}
        />
        <Metric
          label="Leak Watch"
          value={leakWarnings.length ? `${leakWarnings.length} warning` : performance?.status ?? "n/a"}
          valueClassName={leakWarnings.length ? "text-[var(--warning)]" : undefined}
        />
      </div>

      <div className="mt-3 grid gap-2 lg:grid-cols-2">
        <EnvironmentList
          empty="No display rows reported."
          items={environment.displays.map((display) => ({
            key: `${display.index}-${display.name}`,
            title: `${display.name}${display.isMain ? " main" : ""}`,
            detail: formatDisplayFrame(display),
          }))}
          title="Displays"
        />
        <EnvironmentList
          empty="No IDE windows reported."
          items={environment.ideWindows.map((window, index) => ({
            key: `${window.appName}-${window.title}-${index}`,
            title: window.appName,
            detail: window.title,
          }))}
          title="IDE Windows"
        />
        <EnvironmentList
          empty="No Node.js server ports detected."
          items={environment.nodeServers.map((server) => ({
            key: `${server.pid}-${server.port}`,
            title: `${server.processName} :${server.port}`,
            detail: `pid ${server.pid} ${server.listenAddress}`,
          }))}
          title="Node Servers"
        />
        <EnvironmentList
          empty="No compiler process detected."
          items={environment.compilerProcesses.map((process) => ({
            key: `${process.pid}-${process.processName}`,
            title: `${process.processName} pid ${process.pid}`,
            detail: `${formatBytes(process.residentMemoryBytes)} ${process.command}`,
          }))}
          title="Compilers"
        />
        {performance ? (
          <EnvironmentList
            empty={`No helper above ${formatBytes(performance.memoryWarningThresholdBytes)}.`}
            items={(leakWarnings.length ? leakWarnings : performance.monitoredProcesses).map((process) => ({
              key: `${process.pid}-${process.processName}-performance`,
              title: `${process.processName} pid ${process.pid}`,
              detail: `${formatBytes(process.residentMemoryBytes)} ${"detail" in process ? process.detail : process.category}`,
            }))}
            title="Leak Watch"
          />
        ) : null}
      </div>

      <p className="mt-3 text-xs leading-5 text-[var(--foreground-muted)]">
        {firstNodeServer
          ? `${firstNodeServer.processName} is listening on ${firstNodeServer.listenAddress}. `
          : ""}
        {firstCompiler
          ? `${firstCompiler.processName} is using ${formatBytes(firstCompiler.residentMemoryBytes)} RSS. `
          : ""}
        {leakWarnings.length
          ? `${leakWarnings.length} allowlisted helper exceeded the recycling threshold. `
          : ""}
        {environment.probeStatus
          .map((probe) => `${probe.name}: ${probe.status}`)
          .join(" | ")}
      </p>
    </div>
  );
}

function EnvironmentList({
  empty,
  items,
  title,
}: {
  empty: string;
  items: Array<{ key: string; title: string; detail: string }>;
  title: string;
}) {
  return (
    <div className="rounded-[var(--radius-base)] border border-[var(--border-soft)] px-3 py-2.5">
      <p className="text-xs font-semibold text-[var(--foreground)]">{title}</p>
      <div className="mt-2 grid gap-1.5">
        {items.length === 0 ? (
          <p className="text-xs text-[var(--foreground-muted)]">{empty}</p>
        ) : (
          items.slice(0, 4).map((item) => (
            <div className="min-w-0" key={item.key}>
              <p className="truncate text-xs font-medium text-[var(--foreground)]">{item.title}</p>
              <p className="truncate text-xs text-[var(--foreground-muted)]">{item.detail}</p>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

function Metric({
  label,
  value,
  valueClassName,
}: {
  label: string;
  value: string;
  valueClassName?: string;
}) {
  return (
    <div className="rounded-[var(--radius-base)] border border-[var(--border-soft)] px-3 py-2.5">
      <p className="text-xs font-medium text-[var(--foreground-muted)]">{label}</p>
      <p className={`mt-1 text-sm font-semibold text-[var(--foreground)] ${valueClassName ?? ""}`}>
        {value}
      </p>
    </div>
  );
}
