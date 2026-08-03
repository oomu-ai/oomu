import { z } from "zod";
import {
  EVIDENCE_CLASSES,
  parseArtifactId,
  parseChildRunId,
  parseProjectId,
  parseTaskId,
  parseTaskRunId,
  type ArtifactId,
  type ChildRunId,
  type ProjectId,
  type TaskId,
  type TaskRunId,
} from "./p0Contracts";

export const P1_CONTRACT_VERSION = 1 as const;

export const P1_CONTRACT_TYPES = [
  "artifact_workbook",
  "artifact_presentation",
  "desktop_observation",
  "desktop_action",
  "media_asset",
  "remote_device",
  "capability_bundle",
  "learning_candidate",
  "work_graph",
] as const;

const UUID_PATTERN =
  "[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}";
const ZERO_UUID_PATTERN = /_0{8}-0{4}-0{4}-0{4}-0{12}$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const ED25519_SIGNATURE_PATTERN = /^[0-9a-f]{128}$/;
const UTC_TIMESTAMP_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/;

const P1_ID_PREFIXES = {
  observation: "observation",
  action: "action",
  mediaAsset: "media",
  remoteDevice: "device",
  capabilityBundle: "bundle",
  learningCandidate: "learning",
  workGraph: "workgraph",
} as const;

declare const p1IdBrand: unique symbol;
type P1Id<Kind extends keyof typeof P1_ID_PREFIXES> = string & {
  readonly [p1IdBrand]: Kind;
};

export type ObservationId = P1Id<"observation">;
export type DesktopActionId = P1Id<"action">;
export type MediaAssetId = P1Id<"mediaAsset">;
export type RemoteDeviceId = P1Id<"remoteDevice">;
export type CapabilityBundleId = P1Id<"capabilityBundle">;
export type LearningCandidateId = P1Id<"learningCandidate">;
export type WorkGraphId = P1Id<"workGraph">;

function parseP1Id<Kind extends keyof typeof P1_ID_PREFIXES>(
  kind: Kind,
  value: unknown,
): P1Id<Kind> {
  const prefix = P1_ID_PREFIXES[kind];
  const pattern = new RegExp(`^${prefix}_${UUID_PATTERN}$`);
  if (
    typeof value !== "string" ||
    !pattern.test(value) ||
    ZERO_UUID_PATTERN.test(value)
  ) {
    throw new Error(`Invalid ${kind} identifier.`);
  }
  return value as P1Id<Kind>;
}

export function parseObservationId(value: unknown): ObservationId {
  return parseP1Id("observation", value);
}

export function parseDesktopActionId(value: unknown): DesktopActionId {
  return parseP1Id("action", value);
}

export function parseMediaAssetId(value: unknown): MediaAssetId {
  return parseP1Id("mediaAsset", value);
}

export function parseRemoteDeviceId(value: unknown): RemoteDeviceId {
  return parseP1Id("remoteDevice", value);
}

export function parseCapabilityBundleId(value: unknown): CapabilityBundleId {
  return parseP1Id("capabilityBundle", value);
}

export function parseLearningCandidateId(value: unknown): LearningCandidateId {
  return parseP1Id("learningCandidate", value);
}

export function parseWorkGraphId(value: unknown): WorkGraphId {
  return parseP1Id("workGraph", value);
}

function parsedIdSchema<T>(
  parser: (value: unknown) => T,
  label: string,
) {
  return z
    .string()
    .refine((value) => {
      try {
        parser(value);
        return true;
      } catch {
        return false;
      }
    }, `Invalid ${label} identifier.`)
    .transform((value) => parser(value));
}

const projectIdSchema = parsedIdSchema<ProjectId>(parseProjectId, "project");
const taskIdSchema = parsedIdSchema<TaskId>(parseTaskId, "task");
const taskRunIdSchema = parsedIdSchema<TaskRunId>(parseTaskRunId, "task run");
const artifactIdSchema = parsedIdSchema<ArtifactId>(parseArtifactId, "artifact");
const childRunIdSchema = parsedIdSchema<ChildRunId>(parseChildRunId, "child run");
const observationIdSchema = parsedIdSchema<ObservationId>(
  parseObservationId,
  "observation",
);
const desktopActionIdSchema = parsedIdSchema<DesktopActionId>(
  parseDesktopActionId,
  "desktop action",
);
const mediaAssetIdSchema = parsedIdSchema<MediaAssetId>(
  parseMediaAssetId,
  "media asset",
);
const remoteDeviceIdSchema = parsedIdSchema<RemoteDeviceId>(
  parseRemoteDeviceId,
  "remote device",
);
const capabilityBundleIdSchema = parsedIdSchema<CapabilityBundleId>(
  parseCapabilityBundleId,
  "capability bundle",
);
const learningCandidateIdSchema = parsedIdSchema<LearningCandidateId>(
  parseLearningCandidateId,
  "learning candidate",
);
const workGraphIdSchema = parsedIdSchema<WorkGraphId>(parseWorkGraphId, "work graph");

const nonEmptyString = z.string().trim().min(1).max(512);
const opaqueReference = z.string().trim().min(1).max(1024);
const utcTimestamp = z.string().refine(
  (value) => UTC_TIMESTAMP_PATTERN.test(value) && !Number.isNaN(Date.parse(value)),
  "Timestamp must be a valid UTC RFC 3339 value.",
);
const sha256 = z.string().regex(SHA256_PATTERN);

export const evidenceReferenceSchema = z
  .object({
    projectId: projectIdSchema,
    evidenceClass: z.enum(EVIDENCE_CLASSES),
    reference: opaqueReference,
    taskRunId: taskRunIdSchema.optional(),
  })
  .passthrough();

export type P1EvidenceReference = z.infer<typeof evidenceReferenceSchema>;

export const signedEnvelopeSchema = z
  .object({
    algorithm: z.literal("ed25519"),
    keyId: nonEmptyString,
    payloadSha256: sha256,
    signature: z.string().regex(ED25519_SIGNATURE_PATTERN),
    signedAt: utcTimestamp,
  })
  .passthrough();

export type SignedEnvelope = z.infer<typeof signedEnvelopeSchema>;

export const resourceBudgetTelemetrySchema = z
  .object({
    limits: z
      .object({
        tokens: z.number().int().nonnegative(),
        wallTimeMs: z.number().int().nonnegative(),
        memoryBytes: z.number().int().nonnegative(),
        processes: z.number().int().nonnegative(),
        networkRequests: z.number().int().nonnegative(),
        toolCalls: z.number().int().nonnegative(),
        concurrentChildren: z.number().int().min(1).max(8),
        mutations: z.number().int().nonnegative(),
      })
      .passthrough(),
    usage: z
      .object({
        tokens: z.number().int().nonnegative(),
        wallTimeMs: z.number().int().nonnegative(),
        peakMemoryBytes: z.number().int().nonnegative(),
        processes: z.number().int().nonnegative(),
        networkRequests: z.number().int().nonnegative(),
        toolCalls: z.number().int().nonnegative(),
        peakConcurrentChildren: z.number().int().min(0).max(8),
        mutationAttempts: z.number().int().nonnegative(),
        mutationsCommitted: z.number().int().nonnegative(),
      })
      .passthrough(),
    sampledAt: utcTimestamp,
  })
  .passthrough();

export type ResourceBudgetTelemetry = z.infer<
  typeof resourceBudgetTelemetrySchema
>;

function addCrossProjectIssue(
  context: z.RefinementCtx,
  path: (string | number)[],
) {
  context.addIssue({
    code: "custom",
    message: "Cross-project references are not allowed.",
    path,
  });
}

function checkEvidenceProject(
  projectId: ProjectId,
  evidence: readonly P1EvidenceReference[],
  context: z.RefinementCtx,
) {
  evidence.forEach((item, index) => {
    if (item.projectId !== projectId) {
      addCrossProjectIssue(context, ["evidence", index, "projectId"]);
    }
  });
}

const workbookWorksheetSchema = z
  .object({
    sheetId: nonEmptyString,
    projectId: projectIdSchema,
    name: z.string().trim().min(1).max(128),
    rowCount: z.number().int().nonnegative(),
    columnCount: z.number().int().nonnegative(),
  })
  .passthrough();

export const artifactWorkbookSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("artifact_workbook"),
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    artifactId: artifactIdSchema,
    revision: z.number().int().positive(),
    locale: z.string().trim().min(2).max(35),
    dateSystem: z.enum(["1900", "1904"]),
    worksheets: z.array(workbookWorksheetSchema).min(1).max(1024),
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((workbook, context) => {
    checkEvidenceProject(workbook.projectId, workbook.evidence, context);
    workbook.worksheets.forEach((sheet, index) => {
      if (sheet.projectId !== workbook.projectId) {
        addCrossProjectIssue(context, ["worksheets", index, "projectId"]);
      }
    });
  });

export type ArtifactWorkbook = z.infer<typeof artifactWorkbookSchema>;

const presentationSlideSchema = z
  .object({
    slideId: nonEmptyString,
    projectId: projectIdSchema,
    position: z.number().int().nonnegative(),
    layoutId: nonEmptyString,
  })
  .passthrough();

export const artifactPresentationSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("artifact_presentation"),
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    artifactId: artifactIdSchema,
    revision: z.number().int().positive(),
    aspectRatio: z.enum(["16:9", "4:3"]),
    slides: z.array(presentationSlideSchema).min(1).max(1000),
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((presentation, context) => {
    checkEvidenceProject(presentation.projectId, presentation.evidence, context);
    presentation.slides.forEach((slide, index) => {
      if (slide.projectId !== presentation.projectId) {
        addCrossProjectIssue(context, ["slides", index, "projectId"]);
      }
    });
  });

export type ArtifactPresentation = z.infer<typeof artifactPresentationSchema>;

const desktopElementReferenceSchema = z
  .object({
    elementRef: opaqueReference,
    projectId: projectIdSchema,
    role: nonEmptyString,
    secure: z.boolean(),
    expiresAt: utcTimestamp,
  })
  .passthrough();

export const desktopObservationSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("desktop_observation"),
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    observationId: observationIdSchema,
    revision: z.number().int().positive(),
    observedAt: utcTimestamp,
    application: z
      .object({
        bundleId: z.string().trim().min(3).max(255),
        processId: z.number().int().positive(),
        name: nonEmptyString,
      })
      .passthrough(),
    window: z
      .object({
        windowRef: opaqueReference,
        title: z.string().max(512),
      })
      .passthrough(),
    elements: z.array(desktopElementReferenceSchema).max(100_000),
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((observation, context) => {
    checkEvidenceProject(observation.projectId, observation.evidence, context);
    observation.elements.forEach((element, index) => {
      if (element.projectId !== observation.projectId) {
        addCrossProjectIssue(context, ["elements", index, "projectId"]);
      }
    });
  });

export type DesktopObservation = z.infer<typeof desktopObservationSchema>;

export const DESKTOP_ACTION_KINDS = [
  "focus",
  "press",
  "select",
  "type",
  "invoke_menu",
  "scroll",
  "drag_drop",
  "choose_file",
  "apple_event",
] as const;

export const desktopActionSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("desktop_action"),
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    actionId: desktopActionIdSchema,
    observationId: observationIdSchema,
    observationRevision: z.number().int().positive(),
    actionKind: z.enum(DESKTOP_ACTION_KINDS),
    applicationBundleId: z.string().trim().min(3).max(255),
    target: z
      .object({
        projectId: projectIdSchema,
        elementRef: opaqueReference.optional(),
      })
      .passthrough(),
    approvalId: z.string().regex(/^trustgrant_[0-9a-f]{36}$/).optional(),
    requestedAt: utcTimestamp,
    arguments: z.record(z.string(), z.unknown()),
    expectedPostcondition: z
      .object({
        kind: z.enum([
          "element_value",
          "element_state",
          "window_state",
          "file_hash",
          "application_state",
        ]),
        description: z.string().trim().min(1).max(2000),
        evidenceClass: z.enum(EVIDENCE_CLASSES),
        parameters: z.record(z.string(), z.unknown()),
      })
      .passthrough(),
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((action, context) => {
    checkEvidenceProject(action.projectId, action.evidence, context);
    if (action.target.projectId !== action.projectId) {
      addCrossProjectIssue(context, ["target", "projectId"]);
    }
  });

export type DesktopAction = z.infer<typeof desktopActionSchema>;

export const MEDIA_KINDS = ["audio", "image", "video"] as const;

const mediaRelationshipSchema = z
  .object({
    mediaAssetId: mediaAssetIdSchema,
    projectId: projectIdSchema,
    relationship: z.enum(["source", "derivative", "transcript", "thumbnail"]),
  })
  .passthrough();

export const mediaAssetSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("media_asset"),
    projectId: projectIdSchema,
    taskId: taskIdSchema.optional(),
    taskRunId: taskRunIdSchema.optional(),
    mediaAssetId: mediaAssetIdSchema,
    mediaKind: z.enum(MEDIA_KINDS),
    mimeType: z.string().regex(/^[a-z0-9][a-z0-9!#$&^_.+-]*\/[a-z0-9][a-z0-9!#$&^_.+-]*$/i),
    sha256,
    byteLength: z.number().int().nonnegative(),
    createdAt: utcTimestamp,
    source: z
      .object({
        kind: z.enum(["microphone", "voice_message", "screenshot", "clipboard", "camera", "project_file", "generated"]),
        projectId: projectIdSchema,
        reference: opaqueReference,
      })
      .passthrough(),
    retentionPolicy: z
      .object({
        mode: z.enum(["task", "project", "until"]),
        expiresAt: utcTimestamp.optional(),
      })
      .passthrough(),
    redactionPolicy: z
      .object({
        state: z.enum(["not_required", "required", "applied"]),
        categories: z.array(nonEmptyString).max(256),
      })
      .passthrough(),
    providerRoutingPolicy: z
      .object({
        mode: z.enum(["local_only", "approved_providers"]),
        providerIds: z.array(nonEmptyString).max(256),
      })
      .passthrough(),
    relatedAssets: z.array(mediaRelationshipSchema).max(4096),
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((asset, context) => {
    checkEvidenceProject(asset.projectId, asset.evidence, context);
    if (asset.source.projectId !== asset.projectId) {
      addCrossProjectIssue(context, ["source", "projectId"]);
    }
    asset.relatedAssets.forEach((relation, index) => {
      if (relation.projectId !== asset.projectId) {
        addCrossProjectIssue(context, ["relatedAssets", index, "projectId"]);
      }
    });
    if (asset.retentionPolicy.mode === "until" && !asset.retentionPolicy.expiresAt) {
      context.addIssue({
        code: "custom",
        message: "Until-based retention requires an expiry.",
        path: ["retentionPolicy", "expiresAt"],
      });
    }
    if (
      asset.providerRoutingPolicy.mode === "local_only" &&
      asset.providerRoutingPolicy.providerIds.length > 0
    ) {
      context.addIssue({
        code: "custom",
        message: "Local-only media cannot declare remote providers.",
        path: ["providerRoutingPolicy", "providerIds"],
      });
    }
  });

export type MediaAsset = z.infer<typeof mediaAssetSchema>;

export const REMOTE_DEVICE_SCOPES = [
  "create_task",
  "view_task",
  "steer_task",
  "stop_task",
  "answer_clarification",
  "approve_bounded_action",
  "request_artifact",
] as const;

export const remoteDeviceSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("remote_device"),
    remoteDeviceId: remoteDeviceIdSchema,
    label: nonEmptyString,
    publicKey: z.string().regex(SHA256_PATTERN),
    allowedProjectIds: z.array(projectIdSchema).min(1).max(1024),
    scopes: z.array(z.enum(REMOTE_DEVICE_SCOPES)).min(1),
    pairedAt: utcTimestamp,
    expiresAt: utcTimestamp,
    revokedAt: utcTimestamp.optional(),
    evidence: z.array(evidenceReferenceSchema).max(4096),
    signature: signedEnvelopeSchema,
  })
  .passthrough()
  .superRefine((device, context) => {
    const allowed = new Set<ProjectId>(device.allowedProjectIds);
    device.evidence.forEach((item, index) => {
      if (!allowed.has(item.projectId)) {
        addCrossProjectIssue(context, ["evidence", index, "projectId"]);
      }
    });
  });

export type RemoteDevice = z.infer<typeof remoteDeviceSchema>;

export const CAPABILITY_KINDS = [
  "file",
  "network",
  "connector",
  "model",
  "executable",
  "schedule",
  "child_agent",
  "mutation",
] as const;

export const capabilityBundleSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("capability_bundle"),
    capabilityBundleId: capabilityBundleIdSchema,
    name: nonEmptyString,
    packageVersion: z.string().regex(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/),
    publisher: z
      .object({
        id: nonEmptyString,
        name: nonEmptyString,
      })
      .passthrough(),
    scope: z
      .object({
        kind: z.enum(["project", "global"]),
        projectIds: z.array(projectIdSchema).max(1024),
      })
      .passthrough(),
    capabilities: z.array(z.enum(CAPABILITY_KINDS)).max(256),
    requestedGrants: z
      .array(
        z
          .object({
            capability: z.enum(CAPABILITY_KINDS),
            scope: nonEmptyString,
            reason: z.string().trim().min(1).max(2000),
          })
          .passthrough(),
      )
      .max(256),
    payloadSha256: sha256,
    evidence: z.array(evidenceReferenceSchema).max(4096),
    signature: signedEnvelopeSchema,
  })
  .passthrough()
  .superRefine((bundle, context) => {
    if (bundle.scope.kind === "project" && bundle.scope.projectIds.length === 0) {
      context.addIssue({
        code: "custom",
        message: "Project-scoped bundles require at least one Project.",
        path: ["scope", "projectIds"],
      });
    }
    const allowed = new Set<ProjectId>(bundle.scope.projectIds);
    const declaredCapabilities = new Set(bundle.capabilities);
    bundle.evidence.forEach((item, index) => {
      if (bundle.scope.kind === "project" && !allowed.has(item.projectId)) {
        addCrossProjectIssue(context, ["evidence", index, "projectId"]);
      }
    });
    bundle.requestedGrants.forEach((grant, index) => {
      if (!declaredCapabilities.has(grant.capability)) {
        context.addIssue({
          code: "custom",
          message: "Requested grants must be declared capabilities.",
          path: ["requestedGrants", index, "capability"],
        });
      }
    });
    if (bundle.payloadSha256 !== bundle.signature.payloadSha256) {
      context.addIssue({
        code: "custom",
        message: "Bundle signature digest does not match its payload digest.",
        path: ["signature", "payloadSha256"],
      });
    }
  });

export type CapabilityBundle = z.infer<typeof capabilityBundleSchema>;

const learningSourceSchema = z
  .object({
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    evidence: z.array(evidenceReferenceSchema).min(1).max(4096),
  })
  .passthrough();

export const learningCandidateSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("learning_candidate"),
    projectId: projectIdSchema,
    learningCandidateId: learningCandidateIdSchema,
    candidateVersion: z.number().int().positive(),
    candidateKind: z.enum(["procedure", "preference", "correction", "verification_rule", "failure_avoidance"]),
    proposedScope: z.enum(["project", "global_with_confirmation"]),
    summary: z.string().trim().min(1).max(4000),
    proposedDiff: z
      .object({
        base: z.string().max(20_000),
        proposed: z.string().trim().min(1).max(20_000),
        changedFields: z.array(nonEmptyString).min(1).max(256),
      })
      .passthrough(),
    status: z.enum(["proposed", "accepted", "rejected", "postponed"]),
    sourceTasks: z.array(learningSourceSchema).min(1).max(256),
    evidence: z.array(evidenceReferenceSchema).min(1).max(4096),
  })
  .passthrough()
  .superRefine((candidate, context) => {
    checkEvidenceProject(candidate.projectId, candidate.evidence, context);
    candidate.sourceTasks.forEach((source, sourceIndex) => {
      if (source.projectId !== candidate.projectId) {
        addCrossProjectIssue(context, ["sourceTasks", sourceIndex, "projectId"]);
      }
      source.evidence.forEach((item, evidenceIndex) => {
        if (item.projectId !== candidate.projectId) {
          addCrossProjectIssue(context, ["sourceTasks", sourceIndex, "evidence", evidenceIndex, "projectId"]);
        }
      });
    });
  });

export type LearningCandidate = z.infer<typeof learningCandidateSchema>;

export const WORK_GRAPH_NODE_KINDS = [
  "parent",
  "specialist",
  "join",
  "checkpoint",
  "retry",
  "synthesis",
] as const;

const workGraphNodeSchema = z
  .object({
    nodeId: nonEmptyString,
    kind: z.enum(WORK_GRAPH_NODE_KINDS),
    projectId: projectIdSchema,
    dependsOn: z.array(nonEmptyString).max(256),
    childRunId: childRunIdSchema.optional(),
  })
  .passthrough();

export const workGraphSchema = z
  .object({
    schemaVersion: z.literal(P1_CONTRACT_VERSION),
    contractType: z.literal("work_graph"),
    projectId: projectIdSchema,
    taskId: taskIdSchema,
    taskRunId: taskRunIdSchema,
    workGraphId: workGraphIdSchema,
    revision: z.number().int().positive(),
    nodes: z.array(workGraphNodeSchema).min(1).max(256),
    maxConcurrentChildren: z.number().int().min(1).max(8),
    parentOwnsMutations: z.literal(true),
    resourceBudget: resourceBudgetTelemetrySchema,
    evidence: z.array(evidenceReferenceSchema).max(4096),
  })
  .passthrough()
  .superRefine((graph, context) => {
    checkEvidenceProject(graph.projectId, graph.evidence, context);
    const nodeIds = new Set<string>();
    graph.nodes.forEach((node, index) => {
      if (node.projectId !== graph.projectId) {
        addCrossProjectIssue(context, ["nodes", index, "projectId"]);
      }
      if (nodeIds.has(node.nodeId)) {
        context.addIssue({
          code: "custom",
          message: "Work graph node IDs must be unique.",
          path: ["nodes", index, "nodeId"],
        });
      }
      nodeIds.add(node.nodeId);
    });
    graph.nodes.forEach((node, index) => {
      node.dependsOn.forEach((dependency, dependencyIndex) => {
        if (!nodeIds.has(dependency) || dependency === node.nodeId) {
          context.addIssue({
            code: "custom",
            message: "Work graph dependencies must reference another declared node.",
            path: ["nodes", index, "dependsOn", dependencyIndex],
          });
        }
      });
    });

    const nodeById = new Map(graph.nodes.map((node) => [node.nodeId, node]));
    const dependencyCount = new Map<string, number>(
      [...nodeIds].map((nodeId) => [nodeId, 0]),
    );
    const dependents = new Map<string, string[]>();
    nodeById.forEach((node) => {
      node.dependsOn.forEach((dependency) => {
        if (!nodeIds.has(dependency) || dependency === node.nodeId) return;
        dependencyCount.set(node.nodeId, (dependencyCount.get(node.nodeId) ?? 0) + 1);
        dependents.set(dependency, [
          ...(dependents.get(dependency) ?? []),
          node.nodeId,
        ]);
      });
    });
    const ready = [...dependencyCount]
      .filter(([, count]) => count === 0)
      .map(([nodeId]) => nodeId);
    let visitedNodes = 0;
    while (ready.length > 0) {
      const nodeId = ready.pop();
      if (!nodeId) continue;
      visitedNodes += 1;
      for (const dependent of dependents.get(nodeId) ?? []) {
        const remaining = (dependencyCount.get(dependent) ?? 0) - 1;
        dependencyCount.set(dependent, remaining);
        if (remaining === 0) ready.push(dependent);
      }
    }
    if (visitedNodes !== nodeIds.size) {
      context.addIssue({
        code: "custom",
        message: "Work graph dependency cycles are not allowed.",
        path: ["nodes"],
      });
    }

    const { limits, usage } = graph.resourceBudget;
    if (graph.maxConcurrentChildren !== limits.concurrentChildren) {
      context.addIssue({
        code: "custom",
        message: "Work graph concurrency must match its resource budget limit.",
        path: ["maxConcurrentChildren"],
      });
    }
    const boundedUsage: readonly [string, number, number][] = [
      ["tokens", usage.tokens, limits.tokens],
      ["wallTimeMs", usage.wallTimeMs, limits.wallTimeMs],
      ["peakMemoryBytes", usage.peakMemoryBytes, limits.memoryBytes],
      ["processes", usage.processes, limits.processes],
      ["networkRequests", usage.networkRequests, limits.networkRequests],
      ["toolCalls", usage.toolCalls, limits.toolCalls],
      [
        "peakConcurrentChildren",
        usage.peakConcurrentChildren,
        limits.concurrentChildren,
      ],
      ["mutationAttempts", usage.mutationAttempts, limits.mutations],
    ];
    boundedUsage.forEach(([field, used, limit]) => {
      if (used > limit) {
        context.addIssue({
          code: "custom",
          message: "Work graph resource usage exceeds its declared limit.",
          path: ["resourceBudget", "usage", field],
        });
      }
    });
    if (usage.mutationsCommitted > usage.mutationAttempts) {
      context.addIssue({
        code: "custom",
        message: "Committed mutations cannot exceed mutation attempts.",
        path: ["resourceBudget", "usage", "mutationsCommitted"],
      });
    }
  });

export type WorkGraph = z.infer<typeof workGraphSchema>;

export function parseArtifactWorkbook(value: unknown): ArtifactWorkbook {
  return artifactWorkbookSchema.parse(value);
}

export function parseArtifactPresentation(value: unknown): ArtifactPresentation {
  return artifactPresentationSchema.parse(value);
}

export function parseDesktopObservation(value: unknown): DesktopObservation {
  return desktopObservationSchema.parse(value);
}

export function parseDesktopAction(value: unknown): DesktopAction {
  return desktopActionSchema.parse(value);
}

export function parseMediaAsset(value: unknown): MediaAsset {
  return mediaAssetSchema.parse(value);
}

export function parseRemoteDevice(value: unknown): RemoteDevice {
  return remoteDeviceSchema.parse(value);
}

export function parseCapabilityBundle(value: unknown): CapabilityBundle {
  return capabilityBundleSchema.parse(value);
}

export function parseLearningCandidate(value: unknown): LearningCandidate {
  return learningCandidateSchema.parse(value);
}

export function parseWorkGraph(value: unknown): WorkGraph {
  return workGraphSchema.parse(value);
}
