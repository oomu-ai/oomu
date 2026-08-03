import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";
import { redactSensitiveText, redactSensitiveValue, safeErrorMessage } from "../src/lib/redaction";

type EvalCase = {
  id: string;
  prompt: string;
  source: "blueprint" | "realistic";
};

type BridgeResponse = Record<string, unknown> & {
  status?: string;
  workflowIr?: unknown;
  reason?: string;
  missingCapabilities?: unknown[];
  missingCapabilityDetails?: unknown[];
  composedBy?: string;
};

type EvalResult = EvalCase & {
  status: string | undefined;
  schemaValid: boolean;
  contractValid: boolean;
  placeholderFree: boolean;
  approvalGateBeforeDraft: boolean | null;
  composedBy: string | null;
  latencyMs: number | null;
  intentMatch: null;
  reason: string;
  missingCapabilities: unknown[];
  missingCapabilityDetails: unknown[];
  runtimeExecuted: boolean | null;
};

const CONTROLLED_RUNTIME_CASE_ID = "controlled-local-runtime";

const BLUEPRINT_PROMPTS: EvalCase[] = [
  {
    id: "blueprint-directory-summarizer",
    source: "blueprint",
    prompt:
      "Scan a local TaskFlow folder, compress the context, summarize it with Gemma, write a markdown report, and preview it on macOS.",
  },
  {
    id: "blueprint-local-sandbox-log-summarizer",
    source: "blueprint",
    prompt:
      "Read sandbox instructions, summarize the contents locally, ask for approval, then write the executive summary back to the sandbox.",
  },
  {
    id: "blueprint-daily-briefing-system-setup",
    source: "blueprint",
    prompt:
      "Read my local Calendar, create a daily setup brief, notify me if there is useful calendar work, and prepare a Mail draft.",
  },
  {
    id: "blueprint-email-responder",
    source: "blueprint",
    prompt:
      "Read unread Mail messages, draft professional replies with Gemma, ask for approval, then open the reply in Mail.",
  },
  {
    id: "blueprint-calendar-assistant",
    source: "blueprint",
    prompt:
      "Scan upcoming Calendar events, extract the highest-priority briefing points, and show a native macOS notification.",
  },
  {
    id: "blueprint-daily-mail-reminders-scraper",
    source: "blueprint",
    prompt:
      "Read recent Mail and Reminders, summarize urgent messages and open tasks, and write the daily brief to the sandbox.",
  },
];

const REALISTIC_PROMPTS: EvalCase[] = [
  {
    id: CONTROLLED_RUNTIME_CASE_ID,
    source: "realistic",
    prompt:
      "Create a local input-agent-output workflow that summarizes supplied text. Do not use external tools, system actions, permissions, files, network access, or side effects.",
  },
  { id: "observed-mail-draft-approval", source: "realistic", prompt: "Read unread mail, draft replies, and ask me before opening any draft." },
  { id: "email-triage", source: "realistic", prompt: "Triage unread email and summarize anything urgent." },
  { id: "daily-brief", source: "realistic", prompt: "Make a daily brief from calendar events and reminders." },
  { id: "folder-summarize", source: "realistic", prompt: "Summarize a folder of local text notes into a short report." },
  { id: "calendar-notify", source: "realistic", prompt: "Notify me about calendar meetings in the next 12 hours." },
  { id: "mail-draft", source: "realistic", prompt: "Draft a polite reply to the newest unread email but do not send it." },
  { id: "reminder-brief", source: "realistic", prompt: "Read reminders and create a concise list of next actions." },
  { id: "sandbox-read-write", source: "realistic", prompt: "Read a sandbox file, summarize it, and write the summary to another file." },
  { id: "approval-before-write", source: "realistic", prompt: "Create a summary and ask me before writing it to disk." },
  { id: "calendar-email-brief", source: "realistic", prompt: "Use calendar and mail to draft a morning briefing email." },
  { id: "meeting-prep", source: "realistic", prompt: "Prepare notes for upcoming meetings from my local calendar." },
  { id: "inbox-deadlines", source: "realistic", prompt: "Find emails that mention deadlines and summarize them." },
  { id: "task-reminder-notify", source: "realistic", prompt: "Read reminders and show a notification with the most important task." },
  { id: "weekly-local-report", source: "realistic", prompt: "Summarize local project notes into a weekly markdown report." },
  { id: "mail-calendar-priorities", source: "realistic", prompt: "Combine recent mail and calendar events into three priorities." },
  { id: "safe-file-summary", source: "realistic", prompt: "Read a local sandbox file and produce a safe executive summary." },
  { id: "draft-with-review", source: "realistic", prompt: "Draft an email response and require approval before opening Mail." },
  { id: "calendar-free-window", source: "realistic", prompt: "Read my calendar and summarize the next free window." },
  { id: "reminder-report", source: "realistic", prompt: "Write a markdown report of my open reminders." },
  { id: "notify-after-summary", source: "realistic", prompt: "Summarize today's schedule and send a native notification." },
  { id: "folder-risk-summary", source: "realistic", prompt: "Scan a folder and summarize risks, useful variables, and next actions." },
];

const corpus = [...BLUEPRINT_PROMPTS, ...REALISTIC_PROMPTS];

const endpointArgument = argValue("--endpoint") ?? "";
const harnessToken = process.env.OOMU_EVAL_HARNESS_TOKEN?.trim() ?? "";
const outPath = argValue("--out") ??
  path.join(
    process.cwd(),
    "reports",
    `workflow-compose-eval-${new Date().toISOString().replace(/[:.]/g, "-")}.json`,
  );
const dryRun = process.argv.includes("--dry-run");
let endpoint: URL | null = null;

async function main() {
  if (!dryRun && (!endpointArgument || !harnessToken)) {
    throw new Error(
      "Non-dry-run workflow evaluation requires --endpoint and OOMU_EVAL_HARNESS_TOKEN for an authenticated native-runtime harness.",
    );
  }
  if (!dryRun) {
    endpoint = validateHarnessEndpoint(endpointArgument);
    validateHarnessToken(harnessToken);
  }
  const catalog = dryRun
    ? { version: "dry-run", actions: [] }
    : await invokeAuthenticatedHarness("get_workflow_capability_catalog");
  const results: EvalResult[] = [];

  for (const item of corpus) {
    if (dryRun) {
      results.push({
        ...item,
        status: "skipped",
        schemaValid: false,
        contractValid: false,
        placeholderFree: true,
        approvalGateBeforeDraft: null,
        composedBy: null,
        latencyMs: null,
        intentMatch: null,
        reason: "Dry run writes corpus only.",
        missingCapabilities: [],
        missingCapabilityDetails: [],
        runtimeExecuted: null,
      });
      continue;
    }

    const started = performance.now();
    const response = await invokeAuthenticatedHarness("compose_workflow", {
      request: {
        prompt: item.prompt,
        capabilityCatalog: catalog,
        workflowId: `eval-${item.id}`,
        name: item.id
          .split("-")
          .map((part) => part[0]?.toUpperCase() + part.slice(1))
          .join(" "),
      },
    });
    const latencyMs = Math.round(performance.now() - started);
    const schemaValid = isSchemaValidWorkflowIr(response.workflowIr);
    const placeholderFree = isPlaceholderFreeResponse(response);
    const contractValid = isContractValidResponse(response, catalog);
    const approvalGateBeforeDraft =
      item.id === "observed-mail-draft-approval"
        ? hasPermissionBeforeTool(response.workflowIr, "draft_system_email")
        : null;
    const runtimeExecuted =
      item.id === CONTROLLED_RUNTIME_CASE_ID
        ? response.status === "composed" && schemaValid && contractValid
          ? await executeControlledWorkflow(response.workflowIr)
          : false
        : null;
    results.push({
      ...item,
      status: response.status,
      schemaValid,
      contractValid,
      placeholderFree,
      approvalGateBeforeDraft,
      composedBy: response.composedBy ?? null,
      latencyMs,
      intentMatch: null,
      reason: response.reason ?? "",
      missingCapabilities: response.missingCapabilities ?? [],
      missingCapabilityDetails: response.missingCapabilityDetails ?? [],
      runtimeExecuted,
    });
  }

  const validCount = results.filter((result) => result.schemaValid).length;
  const composedCount = results.filter((result) => result.status === "composed").length;
  const contractPassCount = results.filter((result) => result.contractValid).length;
  const placeholderLeakCount = results.filter((result) => result.placeholderFree === false).length;
  const observedRegression = results.find((result) => result.id === "observed-mail-draft-approval");
  const measuredLatencies = results
    .map((result) => result.latencyMs)
    .filter((value): value is number => typeof value === "number");
  const report = {
    generatedAt: new Date().toISOString(),
    endpointClass: dryRun
      ? null
      : endpoint?.protocol === "https:"
        ? "authenticated-https-harness"
        : "authenticated-numeric-loopback-harness",
    corpusSize: corpus.length,
    composedCount,
    schemaValidCount: validCount,
    schemaValidRate: corpus.length ? validCount / corpus.length : 0,
    contractPassCount,
    contractPassRate: corpus.length ? contractPassCount / corpus.length : 0,
    placeholderLeakCount,
    observedMailDraftApprovalPassed:
      observedRegression?.status === "composed" &&
      observedRegression.schemaValid === true &&
      observedRegression.approvalGateBeforeDraft === true,
    controlledRuntimeExecuted:
      results.find((result) => result.id === CONTROLLED_RUNTIME_CASE_ID)?.runtimeExecuted === true,
    intentMatchInstructions:
      "Set each result.intentMatch to true/false after human review; the sprint bar is >= 80% matching intent or one small edit away.",
    latencyMs: {
      min: measuredLatencies.length ? Math.min(...measuredLatencies) : null,
      max: measuredLatencies.length ? Math.max(...measuredLatencies) : null,
      average: measuredLatencies.length
        ? Math.round(measuredLatencies.reduce((sum, value) => sum + value, 0) / measuredLatencies.length)
        : null,
    },
    results,
  };

  await mkdir(path.dirname(outPath), { recursive: true });
  const redactedReport = redactSensitiveValue(report);
  await writeFile(outPath, `${JSON.stringify(redactedReport, null, 2)}\n`, "utf8");
  console.log(`Wrote workflow compose eval report to ${redactSensitiveText(outPath)}`);
  if (dryRun) {
    process.exitCode = 1;
  } else if (
    placeholderLeakCount > 0 ||
    contractPassCount !== corpus.length ||
    report.observedMailDraftApprovalPassed !== true ||
    report.controlledRuntimeExecuted !== true
  ) {
    process.exitCode = 1;
  }
}

async function executeControlledWorkflow(workflowIr: unknown) {
  if (!workflowIr || typeof workflowIr !== "object" || Array.isArray(workflowIr)) {
    return false;
  }
  const ir = workflowIr as Record<string, unknown>;
  const workflowId = typeof ir.workflowId === "string" ? ir.workflowId : "";
  const name = typeof ir.name === "string" ? ir.name : "";
  const nodes = Array.isArray(ir.nodes) ? ir.nodes : [];
  const safeNodes = nodes.every(
    (node) =>
      node &&
      typeof node === "object" &&
      !Array.isArray(node) &&
      ["input", "agent", "output"].includes(String((node as Record<string, unknown>).kind)),
  );
  const inputNode = nodes.find(
    (node) =>
      node &&
      typeof node === "object" &&
      !Array.isArray(node) &&
      (node as Record<string, unknown>).kind === "input",
  ) as Record<string, unknown> | undefined;
  const hasAgent = nodes.some(
    (node) =>
      node &&
      typeof node === "object" &&
      !Array.isArray(node) &&
      (node as Record<string, unknown>).kind === "agent",
  );
  const inputNodeId = typeof inputNode?.id === "string" ? inputNode.id : "";
  if (!workflowId || !name || !safeNodes || !hasAgent || !inputNodeId) return false;

  const now = Date.now();
  let saved = false;
  try {
    const save = await invokeAuthenticatedHarness("save_workflow", {
      request: {
        workflow: {
          id: workflowId,
          name,
          steps: JSON.stringify({ workflowIr, evaluationOnly: true }),
          createdAt: now,
          updatedAt: now,
        },
        visualState: { workflowIr, evaluationOnly: true },
        workflowIr,
        activate: true,
      },
    });
    saved = true;
    const workflowVersion = Number(save.workflowVersion);
    if (!Number.isInteger(workflowVersion) || workflowVersion < 1) return false;
    const execution = await invokeAuthenticatedHarness("run_workflow", {
      request: {
        workflowId,
        workflowVersion,
        preflightMode: "skipped",
        inputs: {
          [inputNodeId]: {
            source: "manual",
            value: {
              text: "OOMU real-component evaluation input: summarize this bounded sentence.",
            },
          },
        },
        outputs: {},
      },
    });
    const instance = execution.instance;
    return (
      instance !== null &&
      typeof instance === "object" &&
      !Array.isArray(instance) &&
      (instance as Record<string, unknown>).status === "Completed"
    );
  } finally {
    if (saved) {
      await invokeAuthenticatedHarness("delete_workflow", { id: workflowId });
    }
  }
}

async function invokeAuthenticatedHarness(command: string, args?: Record<string, unknown>) {
  if (!endpoint) throw new Error("Authenticated harness endpoint is unavailable.");
  let response: Response;
  try {
    response = await fetch(endpoint, {
      method: "POST",
      redirect: "error",
      headers: {
        Authorization: `Bearer ${harnessToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ command, args }),
    });
  } catch {
    throw new Error(`${command} failed before the authenticated harness responded.`);
  }
  if (!response.ok) {
    throw new Error(`${command} failed with HTTP ${response.status}.`);
  }
  const contentLength = Number(response.headers.get("content-length") ?? "0");
  if (Number.isFinite(contentLength) && contentLength > 4 * 1024 * 1024) {
    throw new Error(`${command} returned an oversized response.`);
  }
  let responseText: string;
  try {
    responseText = await readBoundedResponseBody(response);
  } catch {
    throw new Error(`${command} returned an oversized or unreadable response.`);
  }
  try {
    return JSON.parse(responseText) as BridgeResponse;
  } catch {
    throw new Error(`${command} returned an invalid response.`);
  }
}

export async function readBoundedResponseBody(
  response: Response,
  maxBytes = 4 * 1024 * 1024,
) {
  if (!response.body) return "";
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > maxBytes) {
        await reader.cancel("workflow_eval_harness_response_oversized");
        throw new Error("workflow_eval_harness_response_oversized");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(bytes);
}

export function validateHarnessEndpoint(rawEndpoint: string) {
  let parsed: URL;
  try {
    parsed = new URL(rawEndpoint);
  } catch {
    throw new Error("Workflow evaluation endpoint must be a valid harness URL.");
  }
  const numericLoopback =
    parsed.protocol === "http:" &&
    (parsed.hostname === "127.0.0.1" || parsed.hostname === "[::1]" || parsed.hostname === "::1") &&
    Boolean(parsed.port) &&
    Number(parsed.port) > 0;
  if (
    (parsed.protocol !== "https:" && !numericLoopback) ||
    !parsed.hostname ||
    parsed.username ||
    parsed.password ||
    parsed.search ||
    parsed.hash
  ) {
    throw new Error(
      "Workflow evaluation endpoint must use HTTPS or an exact numeric loopback host with an explicit port, without embedded credentials, query parameters, or fragments.",
    );
  }
  return parsed;
}

function validateHarnessToken(token: string) {
  if (!token || token.length > 4096 || /[\u0000-\u001f\u007f]/.test(token)) {
    throw new Error("OOMU_EVAL_HARNESS_TOKEN is invalid.");
  }
}

function isSchemaValidWorkflowIr(value: unknown) {
  if (!value || typeof value !== "object") return false;
  const ir = value as Record<string, unknown>;
  return (
    ir.schemaVersion === "1.0.0" &&
    typeof ir.workflowId === "string" &&
    typeof ir.name === "string" &&
    Array.isArray(ir.nodes) &&
    Array.isArray(ir.edges) &&
    ir.nodes.some((node) => isKind(node, "input")) &&
    ir.nodes.some((node) => isKind(node, "output"))
  );
}

function isKind(value: unknown, kind: string) {
  return Boolean(value && typeof value === "object" && (value as Record<string, unknown>).kind === kind);
}

export function isContractValidResponse(response: BridgeResponse, catalog: unknown) {
  if (response.status === "composed") {
    return isSchemaValidWorkflowIr(response.workflowIr) && isPlaceholderFreeResponse(response);
  }
  if (response.status === "failed") {
    return Boolean(response.reason && isPlaceholderFreeResponse(response));
  }
  if (response.status === "needs_connection") {
    return isActionableNeedsConnection(response, catalog) && isPlaceholderFreeResponse(response);
  }
  return false;
}

function isActionableNeedsConnection(response: BridgeResponse, catalog: unknown) {
  const details = Array.isArray(response.missingCapabilityDetails)
    ? response.missingCapabilityDetails
    : [];
  if (!details.length) return false;
  const actions =
    catalog && typeof catalog === "object" && Array.isArray((catalog as Record<string, unknown>).actions)
      ? ((catalog as Record<string, unknown>).actions as Array<Record<string, unknown>>)
      : [];
  return details.every((detail) => {
    if (!detail || typeof detail !== "object") return false;
    const record = detail as Record<string, unknown>;
    const id = typeof record.id === "string" ? record.id : "";
    const title = typeof record.title === "string" ? record.title : "";
    return actions.some((action) => action.id === id || action.title === title);
  });
}

function isPlaceholderFreeResponse(response: BridgeResponse) {
  const checkedValues = [
    response.reason,
    ...(Array.isArray(response.missingCapabilities) ? response.missingCapabilities : []),
    ...(Array.isArray(response.missingCapabilityDetails)
      ? response.missingCapabilityDetails.flatMap((detail) =>
          detail && typeof detail === "object"
            ? [
                (detail as Record<string, unknown>).id,
                (detail as Record<string, unknown>).title,
                (detail as Record<string, unknown>).reason,
              ]
            : [detail],
        )
      : []),
  ];
  return checkedValues.every((value) => !containsPlaceholder(value));
}

function containsPlaceholder(value: unknown) {
  if (typeof value !== "string") return false;
  const normalized = value.trim().toLowerCase();
  if (!normalized) return false;
  if (
    normalized === "x" ||
    normalized === "y" ||
    normalized === "kind" ||
    normalized === "<capability_name>" ||
    normalized.includes("<capability_name>") ||
    normalized === "connect y to do x." ||
    normalized === "connect y to do x"
  ) {
    return true;
  }
  return normalized
    .split(/[^a-z0-9]+/)
    .some((word) => word === "x" || word === "y");
}

function hasPermissionBeforeTool(value: unknown, toolName: string) {
  if (!value || typeof value !== "object") return false;
  const ir = value as Record<string, unknown>;
  if (!Array.isArray(ir.nodes) || !Array.isArray(ir.edges)) return false;
  const nodes = ir.nodes as Array<Record<string, unknown>>;
  const edges = ir.edges as Array<Record<string, unknown>>;
  const permissionIds = nodes
    .filter((node) => node.kind === "permission")
    .map((node) => node.id)
    .filter((id): id is string => typeof id === "string");
  const target = nodes.find((node) => node.kind === "mcp_tool" && node.toolName === toolName);
  if (!permissionIds.length || typeof target?.id !== "string") return false;

  const adjacency = new Map<string, string[]>();
  for (const edge of edges) {
    if (typeof edge.sourceNodeId !== "string" || typeof edge.targetNodeId !== "string") continue;
    const next = adjacency.get(edge.sourceNodeId) ?? [];
    next.push(edge.targetNodeId);
    adjacency.set(edge.sourceNodeId, next);
  }

  return permissionIds.some((permissionId) => hasPath(permissionId, target.id as string, adjacency));
}

function hasPath(start: string, target: string, adjacency: Map<string, string[]>) {
  const seen = new Set<string>();
  const stack = [start];
  while (stack.length) {
    const current = stack.pop();
    if (!current || seen.has(current)) continue;
    if (current === target) return true;
    seen.add(current);
    stack.push(...(adjacency.get(current) ?? []));
  }
  return false;
}

function argValue(flag: string) {
  const index = process.argv.indexOf(flag);
  return index === -1 ? undefined : process.argv[index + 1];
}

const launchedModuleUrl = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href
  : "";
if (import.meta.url === launchedModuleUrl) {
  void main().catch((error) => {
    console.error(safeErrorMessage(error, "Workflow compose evaluation failed."));
    process.exitCode = 1;
  });
}
