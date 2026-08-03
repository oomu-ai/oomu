const P0_UUID_PATTERN =
  "[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}";

const ID_PREFIXES = {
  project: "project",
  task: "task",
  taskRun: "taskrun",
  artifact: "artifact",
  connector: "connector",
  childRun: "childrun",
} as const;

export const P0_CONTRACT_VERSION = 1 as const;

export const TASK_STATES = [
  "queued",
  "planning",
  "awaiting_approval",
  "running",
  "blocked",
  "completed",
  "failed",
  "cancelled",
] as const;

export const EVIDENCE_CLASSES = [
  "model_assertion",
  "observed_result",
  "executed_mutation",
  "verified_postcondition",
  "signed_artifact",
] as const;

declare const p0IdBrand: unique symbol;
type BrandedId<Kind extends keyof typeof ID_PREFIXES> = string & {
  readonly [p0IdBrand]: Kind;
};

export type ProjectId = BrandedId<"project">;
export type TaskId = BrandedId<"task">;
export type TaskRunId = BrandedId<"taskRun">;
export type ArtifactId = BrandedId<"artifact">;
export type ConnectorId = BrandedId<"connector">;
export type ChildRunId = BrandedId<"childRun">;
export type TaskState = (typeof TASK_STATES)[number];
export type EvidenceClass = (typeof EVIDENCE_CLASSES)[number];

export type P0EventEnvelope = {
  schemaVersion: typeof P0_CONTRACT_VERSION;
  eventType: string;
  projectId: ProjectId;
  taskId: TaskId;
  taskRunId?: TaskRunId;
  correlationId: string;
  sequence: number;
  timestamp: string;
  evidenceClass: EvidenceClass;
  payload: unknown;
};

const envelopeKeys = new Set([
  "schemaVersion",
  "eventType",
  "projectId",
  "taskId",
  "taskRunId",
  "correlationId",
  "sequence",
  "timestamp",
  "evidenceClass",
  "payload",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseId<Kind extends keyof typeof ID_PREFIXES>(
  kind: Kind,
  value: unknown,
): BrandedId<Kind> {
  const prefix = ID_PREFIXES[kind];
  const pattern = new RegExp(`^${prefix}_${P0_UUID_PATTERN}$`);
  if (typeof value !== "string" || !pattern.test(value) || /_0{8}-0{4}-0{4}-0{4}-0{12}$/.test(value)) {
    throw new Error(`Invalid ${kind} identifier.`);
  }
  return value as BrandedId<Kind>;
}

export function parseProjectId(value: unknown): ProjectId {
  return parseId("project", value);
}

export function parseTaskId(value: unknown): TaskId {
  return parseId("task", value);
}

export function parseTaskRunId(value: unknown): TaskRunId {
  return parseId("taskRun", value);
}

export function parseArtifactId(value: unknown): ArtifactId {
  return parseId("artifact", value);
}

export function parseConnectorId(value: unknown): ConnectorId {
  return parseId("connector", value);
}

export function parseChildRunId(value: unknown): ChildRunId {
  return parseId("childRun", value);
}

export function parseTaskState(value: unknown): TaskState {
  if (typeof value !== "string" || !TASK_STATES.includes(value as TaskState)) {
    throw new Error("Unknown P0 task state.");
  }
  return value as TaskState;
}

export function parseEvidenceClass(value: unknown): EvidenceClass {
  if (typeof value !== "string" || !EVIDENCE_CLASSES.includes(value as EvidenceClass)) {
    throw new Error("Unknown P0 evidence class.");
  }
  return value as EvidenceClass;
}

export function parseP0EventEnvelope(value: unknown): P0EventEnvelope {
  if (!isRecord(value)) throw new Error("P0 event envelope must be an object.");
  for (const key of Object.keys(value)) {
    if (!envelopeKeys.has(key)) throw new Error(`Unknown P0 event envelope field: ${key}.`);
  }
  if (value.schemaVersion !== P0_CONTRACT_VERSION) {
    throw new Error("Unsupported P0 event envelope version.");
  }
  if (typeof value.eventType !== "string" || !/^[a-z][a-z0-9_.-]{2,127}$/.test(value.eventType)) {
    throw new Error("Invalid P0 event type.");
  }
  if (
    typeof value.correlationId !== "string" ||
    value.correlationId.length < 1 ||
    value.correlationId.length > 128
  ) {
    throw new Error("Invalid P0 correlation identifier.");
  }
  if (!Number.isSafeInteger(value.sequence) || (value.sequence as number) < 0) {
    throw new Error("Invalid P0 event sequence.");
  }
  if (
    typeof value.timestamp !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/.test(value.timestamp) ||
    Number.isNaN(Date.parse(value.timestamp))
  ) {
    throw new Error("Invalid P0 event timestamp.");
  }
  if (!("payload" in value)) throw new Error("P0 event payload is required.");
  return {
    schemaVersion: P0_CONTRACT_VERSION,
    eventType: value.eventType,
    projectId: parseProjectId(value.projectId),
    taskId: parseTaskId(value.taskId),
    ...(value.taskRunId === undefined ? {} : { taskRunId: parseTaskRunId(value.taskRunId) }),
    correlationId: value.correlationId,
    sequence: value.sequence as number,
    timestamp: value.timestamp,
    evidenceClass: parseEvidenceClass(value.evidenceClass),
    payload: value.payload,
  };
}
