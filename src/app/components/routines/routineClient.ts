import { invoke } from "@/lib/invoke";

export type RoutineProposal = {
  scheduleExpression: string;
  scheduleKind: "one_shot" | "recurring";
  timezone: string;
  normalizedSummary: string;
  nextRunsMs: number[];
};

export type RoutineRecord = {
  routineId: string;
  label: string;
  projectId?: string | null;
  workflowId: string;
  workflowVersion?: number | null;
  scheduleExpression: string;
  scheduleKind: string;
  timezone: string;
  isActive: boolean;
  nextRunAtMs?: number | null;
  nextRunsMs: number[];
  missedRunPolicy: "skip" | "run_once" | "run_each";
  consecutiveFailures: number;
  failureThreshold: number;
  pausedReason?: string | null;
  deliveryTarget: Record<string, unknown>;
  lastStatus?: string | null;
  lastError?: string | null;
  deliveryState?: "delivered" | "retrying" | "needs_review" | null;
  deliveryErrorCode?: string | null;
};

export type RoutineHistoryItem = {
  taskRunId?: string;
  taskId?: string;
  runtimeRecordId?: string;
  executionInstanceId?: string;
  correlationId?: string;
  state: string;
  summary: string;
  lastError?: string | null;
  lastErrorCode?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  scheduledForMs?: number | null;
  runCreatedAtMs?: number;
  scheduleCreatedAtMs?: number;
  scheduleUpdatedAtMs?: number;
  scheduleNextRunAtMs?: number | null;
  effects?: RoutineVerificationEffect[];
  deliveryReceipts?: RoutineDeliveryReceipt[];
  outcome?: "completed_with_declined_actions" | null;
  declinedActions?: string[] | null;
};

type RoutineVerificationEffect = {
  idempotencyKey: string;
  effectKind: string;
  state: string;
  resultDigest?: string | null;
  updatedAtMs: number;
};

type RoutineDeliveryReceipt = {
  receiptId: string;
  platform: string;
  eventKind: string;
  state: string;
  providerReceiptHash?: string | null;
  errorCode?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

export type BackgroundStatus = {
  userEnabled: boolean;
  verifiedActive: boolean;
  state: "off" | "turning_on" | "on_verified" | "needs_attention" | "turning_off";
  registrationState: string;
  registrationBackend: string;
  processState: string;
  registrationGeneration?: string | null;
  processId?: number | null;
  buildNumber: number;
  buildIdentity: string;
  profileClass: string;
  profileGenerationSha256: string;
  heartbeatAtMs?: number | null;
  heartbeatAgeMs?: number | null;
  menuVisible: boolean;
  errorCode?: string | null;
  detail: string;
  checkedAtMs: number;
  recentReceipts: BackgroundRuntimeReceipt[];
};

export type BackgroundRuntimeReceipt = {
  receiptId: string;
  kind: string;
  outcome: string;
  runtimeState: string;
  requestedEnabled: boolean;
  registrationGeneration?: string | null;
  processId?: number | null;
  buildNumber: number;
  buildIdentity: string;
  profileClass: string;
  profileGenerationSha256: string;
  detailCode?: string | null;
  subjectIdHash?: string | null;
  resultDigest?: string | null;
  createdAtMs: number;
};

const BACKGROUND_STATES = new Set([
  "off",
  "turning_on",
  "on_verified",
  "needs_attention",
  "turning_off",
]);

export function isBackgroundStatus(value: unknown): value is BackgroundStatus {
  if (!value || typeof value !== "object") return false;
  const status = value as Partial<BackgroundStatus>;
  const structurallyValid = (
    typeof status.userEnabled === "boolean" &&
    typeof status.verifiedActive === "boolean" &&
    typeof status.state === "string" &&
    BACKGROUND_STATES.has(status.state) &&
    typeof status.checkedAtMs === "number"
  );
  if (!structurallyValid) return false;
  if (status.state !== "on_verified") return status.verifiedActive === false;
  return (
    status.userEnabled === true &&
    status.verifiedActive === true &&
    status.registrationState === "registered" &&
    status.processState === "running" &&
    typeof status.processId === "number" &&
    status.processId > 0 &&
    typeof status.heartbeatAtMs === "number" &&
    status.menuVisible === true &&
    !status.errorCode
  );
}

type RoutineCreateRequest = {
  confirmed: true;
  label: string;
  projectId: string;
  workflowId: string;
  workflowVersion: number;
  scheduleExpression: string;
  scheduleKind: RoutineProposal["scheduleKind"];
  timezone: string;
  activeWindowStartMinute: number | null;
  activeWindowEndMinute: number | null;
  endBoundary: "midnight" | null;
  runOnceAfterCreate: boolean;
  missedRunPolicy: string;
  missedRunCap: number;
  taskTemplate: Record<string, unknown>;
  modelRoute: { mode: "workflow_default" };
  deliveryTarget: Record<string, string>;
  authority: { mode: "reviewed_workflow_scope" };
};

export const routineApi = {
  propose: (text: string, timezone: string) => invoke<RoutineProposal>("propose_routine", { request: { text, timezone } }),
  list: () => invoke<RoutineRecord[]>("list_routines"),
  create: (request: RoutineCreateRequest) => invoke<RoutineRecord>("create_routine", { request }),
  pause: (routineId: string) => invoke<RoutineRecord>("pause_routine", { request: { routineId } }),
  resume: (routineId: string) => invoke<RoutineRecord>("resume_routine", { request: { routineId } }),
  runNow: (routineId: string) => invoke<RoutineRecord>("run_routine_now", { request: { routineId } }),
  duplicate: (routineId: string) => invoke<RoutineRecord>("duplicate_routine", { request: { routineId } }),
  remove: (routineId: string) => invoke<void>("delete_routine", { request: { routineId, confirmed: true } }),
  history: (routineId: string) => invoke<RoutineHistoryItem[]>("get_routine_history", { request: { routineId } }),
  retryDelivery: (routineId: string) => invoke<RoutineRecord>("retry_routine_delivery", { request: { routineId, confirmedAbsent: true } }),
  background: () => invoke<BackgroundStatus>("get_background_service_status"),
  setBackground: (enabled: boolean) => invoke<BackgroundStatus>("set_background_service_enabled", { enabled }),
  openBackgroundLoginItems: () => invoke<void>("open_background_login_items_settings"),
};
