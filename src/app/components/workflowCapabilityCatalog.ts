"use client";

import { invoke } from "@/lib/invoke";
import {
  humanizeToolName,
  type WorkflowIrTemplateExample,
} from "./workflowLibrary";
import type { WorkflowIr, WorkflowIrNode } from "./workflowIr";

type WorkflowCapabilityKind =
  | "agent"
  | "control"
  | "mcp_tool"
  | "system_action";

type WorkflowCapabilityAvailability =
  | "available"
  | "requires_connection";

export type WorkflowCapabilityAction = {
  id: string;
  kind: WorkflowCapabilityKind;
  title: string;
  titleKey?: string;
  outcome: string;
  outcomeKey?: string;
  detail: string;
  detailKey?: string;
  copyParams?: Record<string, string | number>;
  source: "native" | "mcp" | "library" | "template";
  available: boolean;
  availability: WorkflowCapabilityAvailability;
  unavailableReason?: string;
  serverName?: string;
  toolName?: string;
  inputSchema?: unknown;
  outputSchema?: unknown;
  nodeKind?: WorkflowIrNode["kind"];
  nodeTemplate?: Partial<WorkflowIrNode>;
  templateId?: string;
};

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export type CapabilityCatalog = {
  authoringEnabled: boolean;
  generatedAtMs: number;
  actions: WorkflowCapabilityAction[];
  templates: WorkflowTemplateCatalogExample[];
  version: string;
};

type WorkflowTemplateCatalogExample = Pick<
  WorkflowIrTemplateExample,
  "description" | "id" | "name" | "seedPrompt" | "workflowIr"
>;

type ComposeWorkflowStatus =
  | "composed"
  | "needs_connection"
  | "failed"
  | "disabled";

type ComposeWorkflowRequest = {
  prompt: string;
  catalog?: CapabilityCatalog;
  projectId?: string | null;
  workflowId?: string;
  name?: string;
};

type EditWorkflowRequest = {
  catalog?: CapabilityCatalog;
  instruction: string;
  workflowIr: WorkflowIr;
};

export type ComposeWorkflowResponse = {
  status: ComposeWorkflowStatus;
  reason: string;
  workflowIr?: WorkflowIr | null;
  partialDraft?: unknown;
  missingCapabilities: string[];
  missingCapabilityDetails?: MissingCapabilityDetail[];
  composedBy?: string;
  attempts: number;
  latencyMs: number;
};

export type MissingCapabilityDetail = {
  id: string;
  title: string;
  outcome: string;
  reason: string;
  source: string;
  serverName?: string;
  toolName?: string;
};

export async function loadWorkflowCapabilityCatalog(): Promise<CapabilityCatalog> {
  return invoke<CapabilityCatalog>("get_workflow_capability_catalog");
}

export function flattenCapabilityCatalog(catalog: CapabilityCatalog) {
  return [...catalog.actions].sort((left, right) =>
    left.title.localeCompare(right.title),
  );
}

export function localizeCapabilityCatalog(
  catalog: CapabilityCatalog,
  t: TranslateFn,
): CapabilityCatalog {
  return {
    ...catalog,
    actions: catalog.actions.map((action) => localizeCapabilityAction(action, t)),
  };
}

export function localizeCapabilityAction(
  action: WorkflowCapabilityAction,
  t: TranslateFn,
): WorkflowCapabilityAction {
  const fallbackCopy = capabilityCopyForAction(action);
  const params = {
    server: friendlyServerName(action.serverName),
    tool: humanizeToolName(action.toolName ?? ""),
    command: commandPreview(action),
    ...(action.copyParams ?? {}),
  };
  return {
    ...action,
    title: translateCopy(t, action.titleKey ?? fallbackCopy.titleKey, action.title, params),
    outcome: translateCopy(
      t,
      action.outcomeKey ?? fallbackCopy.outcomeKey,
      action.outcome,
      params,
    ),
    detail: translateCopy(
      t,
      action.detailKey ?? fallbackCopy.detailKey,
      action.detail,
      params,
    ),
  };
}

export async function composeWorkflowFromNaturalLanguage({
  prompt,
  catalog,
  projectId,
  workflowId,
  name,
}: ComposeWorkflowRequest): Promise<ComposeWorkflowResponse> {
  const resolvedCatalog = catalog ?? (await loadWorkflowCapabilityCatalog());
  return invoke<ComposeWorkflowResponse>("compose_workflow", {
    request: {
      prompt,
      capabilityCatalog: resolvedCatalog,
      projectId: projectId || null,
      workflowId,
      name,
    },
  });
}

export async function editWorkflowFromNaturalLanguage({
  catalog,
  instruction,
  workflowIr,
}: EditWorkflowRequest): Promise<ComposeWorkflowResponse> {
  const resolvedCatalog = catalog ?? (await loadWorkflowCapabilityCatalog());
  return invoke<ComposeWorkflowResponse>("edit_workflow", {
    request: {
      instruction,
      workflowIr,
      capabilityCatalog: resolvedCatalog,
    },
  });
}

function capabilityCopyForAction(action: WorkflowCapabilityAction) {
  if (action.kind === "mcp_tool" && action.serverName && action.toolName) {
    return mcpCapabilityCopy(action.serverName, action.toolName);
  }
  if (action.kind === "system_action") {
    return systemActionCapabilityCopy(action.nodeTemplate);
  }
  if (action.kind === "agent") {
    return agentCapabilityCopy(action.nodeTemplate);
  }
  return {
    titleKey: "workflows.capabilities.generic.title",
    outcomeKey: "workflows.capabilities.generic.outcome",
    detailKey: "workflows.capabilities.generic.detail",
    title: action.title,
    outcome: action.outcome,
    detail: action.detail,
  };
}

function mcpCapabilityCopy(serverName: string, toolName: string) {
  const key = `${serverName}.${toolName}`;
  const known = {
    "local_filesystem.list_directory": {
      title: "List files in the workflow folder",
      outcome: "See what files are available in the local workflow folder.",
      detail: "Reads filenames from the local workflow folder.",
    },
    "local_filesystem.read_file": {
      title: "Read a file from the workflow folder",
      outcome: "Read a local file that the workflow is allowed to use.",
      detail: "Reads text from the local workflow folder.",
    },
    "local_filesystem.write_file": {
      title: "Write a file to the workflow folder",
      outcome: "Save generated text into the local workflow folder.",
      detail: "Writes a reviewed file into the local workflow folder.",
    },
    "macos_applescript.read_system_calendar": {
      title: "Read your Calendar",
      outcome: "Read upcoming events from Calendar on this Mac.",
      detail: "Reads local Calendar events.",
    },
    "macos_applescript.trigger_system_notification": {
      title: "Show a Mac notification",
      outcome: "Display a native notification on this Mac.",
      detail: "Shows a local notification.",
    },
    "macos_applescript.draft_system_email": {
      title: "Open a Mail draft for review",
      outcome: "Prepare a visible Apple Mail draft that you can review before sending.",
      detail: "Opens a local Mail draft.",
    },
    "macos_applescript.read_system_emails": {
      title: "Read your Mail",
      outcome: "Read recent messages from Mail on this Mac.",
      detail: "Reads local Mail messages.",
    },
    "macos_applescript.read_system_reminders": {
      title: "Read your Reminders",
      outcome: "Read tasks from Reminders on this Mac.",
      detail: "Reads local Reminders tasks.",
    },
    "taskflow_native.folder_read": {
      title: "Read an approved project folder",
      outcome: "Scan text files from a folder you have approved for the workflow.",
      detail: "Reads files from an approved project folder.",
    },
    "taskflow_native.write_markdown_report": {
      title: "Write a project report",
      outcome: "Save a Markdown report in the approved project folder.",
      detail: "Writes a report into the approved project folder.",
    },
    "taskflow_native.preview_report": {
      title: "Open the report for review",
      outcome: "Open the generated report on this Mac so you can inspect it.",
      detail: "Opens a local report preview.",
    },
  }[key];
  const suffix = key.replace(/[^a-zA-Z0-9]+/g, "_");
  if (known) {
    return {
      ...known,
      titleKey: `workflows.capabilities.mcp.${suffix}.title`,
      outcomeKey: `workflows.capabilities.mcp.${suffix}.outcome`,
      detailKey: `workflows.capabilities.mcp.${suffix}.detail`,
    };
  }
  return {
    titleKey: "workflows.capabilities.mcp.generic.title",
    outcomeKey: "workflows.capabilities.mcp.generic.outcome",
    detailKey: "workflows.capabilities.mcp.generic.detail",
    title: humanizeToolName(toolName),
    outcome: `Use ${humanizeToolName(toolName)} from ${friendlyServerName(serverName)}.`,
    detail: `Uses ${humanizeToolName(toolName)} from ${friendlyServerName(serverName)}.`,
  };
}

function systemActionCapabilityCopy(nodeTemplate: unknown) {
  const template =
    nodeTemplate && typeof nodeTemplate === "object"
      ? (nodeTemplate as Partial<WorkflowIrNode>)
      : {};
  const command =
    "command" in template && typeof template.command === "string"
      ? template.command
      : "";
  if (command === "open") {
    return {
      titleKey: "workflows.capabilities.system.open.title",
      outcomeKey: "workflows.capabilities.system.open.outcome",
      detailKey: "workflows.capabilities.system.open.detail",
      title: "Open something on this Mac",
      outcome: "Open a generated file or draft so you can review it.",
      detail: "Opens the result locally for review.",
    };
  }
  return {
    titleKey: "workflows.capabilities.system.generic.title",
    outcomeKey: "workflows.capabilities.system.generic.outcome",
    detailKey: "workflows.capabilities.system.generic.detail",
    title: "Use a local action",
    outcome: "Run a limited local action that is already part of the workflow.",
    detail: "Uses a local action with the workflow's saved settings.",
  };
}

function agentCapabilityCopy(nodeTemplate: unknown) {
  const template =
    nodeTemplate && typeof nodeTemplate === "object"
      ? (nodeTemplate as Partial<Extract<WorkflowIrNode, { kind: "agent" }>>)
      : {};
  const text = `${template.label ?? ""} ${template.objective ?? ""}`.toLowerCase();
  if (/(draft|reply|email|message)/.test(text)) {
    return {
      titleKey: "workflows.capabilities.agent.draft_message.title",
      outcomeKey: "workflows.capabilities.agent.draft_message.outcome",
      detailKey: "workflows.capabilities.agent.draft_message.detail",
      title: "Draft a message",
      outcome: "Write a reply or note from what the workflow found.",
      detail: "Creates text for you to review before a later step uses it.",
    };
  }
  if (/(decide|recommend|priority|risk|route|branch)/.test(text)) {
    return {
      titleKey: "workflows.capabilities.agent.decide.title",
      outcomeKey: "workflows.capabilities.agent.decide.outcome",
      detailKey: "workflows.capabilities.agent.decide.detail",
      title: "Summarize and suggest what to do next",
      outcome: "Turn what the workflow found into a short summary and a suggested next step.",
      detail: "Helps review the information before a later action runs.",
    };
  }
  if (/(summar|brief|report|compress|capture|audit)/.test(text)) {
    return {
      titleKey: "workflows.capabilities.agent.summarize.title",
      outcomeKey: "workflows.capabilities.agent.summarize.outcome",
      detailKey: "workflows.capabilities.agent.summarize.detail",
      title: "Summarize what it found",
      outcome: "Turn the information from earlier steps into a concise summary.",
      detail: "Keeps the summary tied to the workflow's inputs.",
    };
  }
  return {
    titleKey: "workflows.capabilities.agent.generic.title",
    outcomeKey: "workflows.capabilities.agent.generic.outcome",
    detailKey: "workflows.capabilities.agent.generic.detail",
    title: "Think through the next step",
    outcome: "Use the workflow context to prepare the next readable step.",
    detail: "Transforms earlier results into the next workflow result.",
  };
}

function translateCopy(
  t: TranslateFn,
  key: string | undefined,
  fallback: string,
  variables: Record<string, string | number>,
) {
  if (!key) {
    return fallback;
  }
  const translated = t(key, variables);
  return translated === key ? fallback : translated;
}

function friendlyServerName(serverName?: string) {
  if (!serverName) {
    return "";
  }
  return humanizeToolName(serverName.replace(/^macos_/, "macOS "));
}

function commandPreview(action: WorkflowCapabilityAction) {
  const template = action.nodeTemplate;
  if (!template || typeof template !== "object" || !("command" in template)) {
    return "";
  }
  const command =
    typeof template.command === "string" ? template.command : "";
  const args = Array.isArray(template.args) ? template.args.map(String) : [];
  return [command, ...args].filter(Boolean).join(" ");
}
