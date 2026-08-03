"use client";

import { invoke } from "@/lib/invoke";
import {
  workflowIrSchema,
  type WorkflowIr,
} from "./workflowIr";
import type { WorkflowReviewCapabilities } from "./routines/workflowReviewCapabilities";

export type SavedWorkflow = {
  id: string;
  name: string;
  description: string;
  projectId?: string | null;
  isActive: boolean;
  lastRunAt?: number;
  workflowIr: WorkflowIr;
  workflowVersion?: number;
  compilationStatus?: WorkflowBlueprintStatus;
  reviewCapabilities?: WorkflowReviewCapabilities;
  createdAt: number;
  updatedAt: number;
};

type WorkflowRow = {
  id: string;
  name: string;
  steps: string;
  projectId?: string | null;
  project_id?: string | null;
  workflowVersion?: number | null;
  compilationStatus?: WorkflowBlueprintStatus | null;
  reviewCapabilities?: WorkflowReviewCapabilities;
  createdAt?: number;
  updatedAt?: number;
  created_at?: number;
  updated_at?: number;
};

type WorkflowStepsPayload = {
  description?: unknown;
  lastRunAt?: unknown;
  isActive?: unknown;
  workflowIr?: unknown;
  workflowVersion?: unknown;
  compilationStatus?: unknown;
  projectId?: unknown;
  oomuArtifactProvenance?: unknown;
};

export type SaveWorkflowResponse = {
  workflowId: string;
  workflowVersion: number;
  compilationStatus: "Compiled";
  compiledNodeCount: number;
  projectId: string | null;
  reviewCapabilities: WorkflowReviewCapabilities;
};

type WorkflowBlueprintStatus =
  | "Draft"
  | "Compiling"
  | "Compiled"
  | "Failed";

type SavedWorkflowIr = {
  workflowId: string;
  workflowVersion: number;
  name: string;
  description: string;
  visualState: unknown;
  workflowIr: WorkflowIr;
  workflow: SavedWorkflow;
  compilationStatus: WorkflowBlueprintStatus;
  compilationError?: string | null;
  isActive: boolean;
  createdAt: number;
  updatedAt: number;
  compiledAt?: number | null;
};

type WorkflowIrRow = {
  workflowId: string;
  version: number;
  name: string;
  description?: string;
  visualState: unknown;
  workflowIr?: unknown;
  compilationStatus: WorkflowBlueprintStatus;
  compilationError?: string | null;
  isActive: boolean;
  createdAtMs: number;
  updatedAtMs: number;
  compiledAtMs?: number | null;
  projectId?: string | null;
};

export type CompiledInstruction = {
  id: string;
  workflowId: string;
  workflowVersion: number;
  nodeId: string;
  nodeKind: "agent";
  systemPrompt: string;
  inputVariableMappings: Record<string, string>;
  evaluationProtocol: unknown;
  compilerModel: string;
  compilerVersion: string;
  createdAtMs: number;
};

type ExecutionStatus =
  | "Pending"
  | "Running"
  | "AwaitingApproval"
  | "Completed"
  | "Failed";

export type ApprovalRequest = {
  instanceId: string;
  workflowId: string;
  nodeId: string;
  message: string;
  context: unknown;
  approvalToken: string;
  approveCommand: unknown;
  rejectCommand: unknown;
};

export type WorkflowNodePayload = {
  status: ExecutionStatus;
  input?: unknown;
  output?: unknown;
  error?: unknown;
  latencyMs?: number;
  promptTokens?: number;
  completionTokens?: number;
};

export type WorkflowCompletion = {
  kind: "empty_collection";
};

export type WorkflowRunResponse = {
  instance: {
    id: string;
    workflowId: string;
    workflowVersion: number;
    status: ExecutionStatus;
    activeNodeId?: string | null;
    outputPayload?: unknown;
    error?: unknown;
    nodePayloads: Record<string, WorkflowNodePayload>;
  };
  executionOrder: string[];
  approvalRequest?: ApprovalRequest;
  completion?: WorkflowCompletion;
};

export type WorkflowPreflightMode = "skipped" | "taskflow_audit";

export async function loadSavedWorkflows() {
  const rows = await invoke<WorkflowRow[]>("get_workflows");
  return rows.flatMap(workflowFromRow);
}

export async function loadSavedWorkflowIrs() {
  const rows = await invoke<WorkflowIrRow[]>("get_workflow_irs");
  return rows.flatMap(workflowIrFromRow);
}

export async function persistWorkflow(workflow: SavedWorkflow) {
  if (!workflow.workflowIr) {
    throw new Error("Workflow saving now requires editable steps.");
  }
  return persistWorkflowIr(workflow, workflow.workflowIr);
}

export async function persistWorkflowIr(
  workflow: SavedWorkflow,
  workflowIr: WorkflowIr,
  visualState?: unknown,
) {
  const parsedWorkflowIr = workflowIrSchema.parse({
    ...workflowIr,
    workflowId: workflow.id,
    name: workflow.name,
    description: workflow.description,
  });
  const resolvedVisualState =
    visualState ??
    workflowVisualState({
      ...workflow,
      workflowIr: parsedWorkflowIr,
      workflowVersion: parsedWorkflowIr.workflowVersion,
    });
  return invoke<SaveWorkflowResponse>("save_workflow", {
    request: {
      projectId: workflow.projectId || null,
      workflow: {
        id: workflow.id,
        name: workflow.name,
        steps: JSON.stringify(resolvedVisualState),
        createdAt: workflow.createdAt,
        updatedAt: workflow.updatedAt,
      },
      visualState: resolvedVisualState,
      workflowIr: parsedWorkflowIr,
      activate: workflow.isActive,
    },
  });
}

function workflowVisualState(workflow: SavedWorkflow) {
  const visualState = {
    description: workflow.description,
    lastRunAt: workflow.lastRunAt,
    isActive: workflow.isActive,
    workflowIr: workflow.workflowIr,
    workflowVersion: workflow.workflowVersion,
    compilationStatus: workflow.compilationStatus,
    projectId: workflow.projectId ?? null,
  };
  return visualState;
}

export async function executeCompiledWorkflow(
  workflowId: string,
  workflowVersion: number | undefined,
  inputNodeId: string,
  value: unknown,
  preflightMode: WorkflowPreflightMode = "skipped",
) {
  return invoke<WorkflowRunResponse>("run_workflow", {
    request: {
      workflowId,
      workflowVersion,
      preflightMode,
      inputs: {
        [inputNodeId]: {
          source: "manual",
          value,
        },
      },
      outputs: {},
    },
  });
}

export async function markWorkflowLastRun(
  workflowId: string,
  lastRunAt: number,
) {
  return invoke<boolean>("update_workflow_last_run", {
    id: workflowId,
    lastRunAt,
  });
}

export async function resolveWorkflowPermission(
  approval: ApprovalRequest,
  decision: "approve" | "reject",
) {
  return invoke<WorkflowRunResponse>("resolve_workflow_permission", {
    request: {
      instanceId: approval.instanceId,
      approvalToken: approval.approvalToken,
      decision,
    },
  });
}

export async function loadCompiledInstructions(
  workflowId: string,
  workflowVersion?: number,
) {
  return invoke<CompiledInstruction[]>("get_compiled_instructions", {
    request: { workflowId, workflowVersion },
  });
}

export async function overrideCompiledInstruction(
  instruction: CompiledInstruction,
  systemPrompt: string,
) {
  return invoke<CompiledInstruction>("update_compiled_instruction", {
    request: {
      workflowId: instruction.workflowId,
      workflowVersion: instruction.workflowVersion,
      nodeId: instruction.nodeId,
      systemPrompt,
    },
  });
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

export async function removeSavedWorkflow(id: string) {
  await invoke<boolean>("delete_workflow", { id });
}

function workflowFromRow(row: WorkflowRow): SavedWorkflow[] {
  const payload = parseWorkflowSteps(row.steps);
  const createdAt = Number(row.createdAt ?? row.created_at ?? new Date().getTime());
  const updatedAt = Number(row.updatedAt ?? row.updated_at ?? createdAt);
  const parsedWorkflowIr = workflowIrSchema.safeParse(payload.workflowIr);
  if (!parsedWorkflowIr.success) {
    return [];
  }
  const workflowVersion =
    typeof row.workflowVersion === "number"
      ? row.workflowVersion
      : typeof payload.workflowVersion === "number"
      ? payload.workflowVersion
      : parsedWorkflowIr.data.workflowVersion;
  const compilationStatus =
    row.compilationStatus === "Draft" ||
    row.compilationStatus === "Compiling" ||
    row.compilationStatus === "Compiled" ||
    row.compilationStatus === "Failed"
      ? row.compilationStatus
      : payload.compilationStatus === "Draft" ||
    payload.compilationStatus === "Compiling" ||
    payload.compilationStatus === "Compiled" ||
    payload.compilationStatus === "Failed"
      ? payload.compilationStatus
      : undefined;

  return [{
    id: row.id,
    name: row.name,
    description:
      typeof payload.description === "string"
        ? payload.description
        : "A custom sequence for your assistant.",
    projectId:
      row.projectId ??
      row.project_id ??
      (typeof payload.projectId === "string" ? payload.projectId : null),
    isActive: typeof payload.isActive === "boolean" ? payload.isActive : true,
    lastRunAt: typeof payload.lastRunAt === "number" ? payload.lastRunAt : undefined,
    workflowIr: parsedWorkflowIr.data,
    workflowVersion,
    ...(compilationStatus === undefined ? {} : { compilationStatus }),
    ...(row.reviewCapabilities === undefined
      ? {}
      : { reviewCapabilities: row.reviewCapabilities }),
    createdAt,
    updatedAt,
  }];
}

function workflowIrFromRow(row: WorkflowIrRow): SavedWorkflowIr[] {
  const parsed = workflowIrSchema.safeParse(row.workflowIr);
  if (!parsed.success) {
    return [];
  }

  const payload = asRecord(row.visualState) as WorkflowStepsPayload;
  const workflow: SavedWorkflow = {
    id: row.workflowId,
    name: row.name,
    description:
      typeof payload.description === "string"
        ? payload.description
        : row.description ?? "A custom sequence for your assistant.",
    projectId: row.projectId ?? null,
    isActive: typeof payload.isActive === "boolean" ? payload.isActive : row.isActive,
    lastRunAt: typeof payload.lastRunAt === "number" ? payload.lastRunAt : undefined,
    workflowIr: parsed.data,
    workflowVersion: row.version,
    compilationStatus: row.compilationStatus,
    createdAt: Number(row.createdAtMs),
    updatedAt: Number(row.updatedAtMs),
  };

  return [
    {
      workflowId: row.workflowId,
      workflowVersion: row.version,
      name: row.name,
      description: row.description ?? "",
      visualState: row.visualState,
      workflowIr: parsed.data,
      workflow,
      compilationStatus: row.compilationStatus,
      compilationError: row.compilationError ?? null,
      isActive: row.isActive,
      createdAt: Number(row.createdAtMs),
      updatedAt: Number(row.updatedAtMs),
      compiledAt: row.compiledAtMs ?? null,
    },
  ];
}

function parseWorkflowSteps(steps: string): WorkflowStepsPayload {
  try {
    const parsed = JSON.parse(steps);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}
