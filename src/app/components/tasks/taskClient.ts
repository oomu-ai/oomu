import { invoke } from "@/lib/invoke";
import type { P0EventEnvelope, TaskState } from "@/lib/p0Contracts";

export type TaskRun = {
  taskRunId: string;
  taskId: string;
  projectId: string | null;
  runtimeKind: string;
  runtimeRecordId: string;
  state: TaskState;
  origin: string;
  correlationId: string;
  summary: string;
  lastError: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  completedAtMs: number | null;
  acknowledgedAtMs: number | null;
  recoveryState: string;
  effectVerificationRequired: boolean;
  validControls: Array<"cancel" | "resume" | "retry" | "acknowledge_failure">;
};

type ResolveTaskEffectVerificationBase = {
  taskRunId: string;
  taskId: string;
  runtimeRecordId: string;
};

type ResolveTaskEffectVerificationIdentity = {
  verificationSequence: number;
  nodeId: string;
  idempotencyKey: string;
  effectKind: string;
};

type ResolveTaskEffectVerificationWithoutIdentity = {
  verificationSequence?: never;
  nodeId?: never;
  idempotencyKey?: never;
  effectKind?: never;
};

type ResolveTaskEffectVerificationRequest = ResolveTaskEffectVerificationBase & (
  | (ResolveTaskEffectVerificationIdentity & {
      decision: "did_not_happen" | "happened";
    })
  | ({ decision: "stop_without_repeating" } & (
      | ResolveTaskEffectVerificationIdentity
      | ResolveTaskEffectVerificationWithoutIdentity
    ))
);

type TaskRecoveryReport = {
  inspected: number;
  reconciled: number;
  lost: number;
  runtimeUnavailable: number;
};

export const taskApi = {
  list: (projectId?: string, state?: TaskState) => invoke<TaskRun[]>("list_task_runs", { filter: { projectId: projectId || null, state: state || null, origin: null, runtimeKind: null, fromMs: null, toMs: null } }),
  reconcile: () => invoke<TaskRecoveryReport>("reconcile_task_runs"),
  control: (command: "cancel_task_run" | "resume_task_run" | "retry_task_run" | "acknowledge_task_failure", taskRunId: string) => invoke<TaskRun>(command, { request: { taskRunId } }),
  resumeAgent: (executionId: string) => invoke("resume_agent_execution", { request: { executionId } }),
  events: (taskRunId: string, afterSequence?: number) => invoke<P0EventEnvelope[]>("reconnect_task_events", { request: { taskRunId, afterSequence } }),
  resolveEffectVerification: (request: ResolveTaskEffectVerificationRequest) =>
    invoke<TaskRun>("resolve_task_effect_verification", { request }),
};
