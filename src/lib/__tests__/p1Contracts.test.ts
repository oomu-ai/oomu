import { describe, expect, it } from "vitest";
import vectors from "../../../schemas/p1-contract-v1-vectors.json";
import {
  CAPABILITY_KINDS,
  DESKTOP_ACTION_KINDS,
  MEDIA_KINDS,
  P1_CONTRACT_TYPES,
  P1_CONTRACT_VERSION,
  REMOTE_DEVICE_SCOPES,
  WORK_GRAPH_NODE_KINDS,
  artifactPresentationSchema,
  artifactWorkbookSchema,
  capabilityBundleSchema,
  desktopActionSchema,
  desktopObservationSchema,
  evidenceReferenceSchema,
  learningCandidateSchema,
  mediaAssetSchema,
  parseArtifactPresentation,
  parseArtifactWorkbook,
  parseCapabilityBundle,
  parseCapabilityBundleId,
  parseDesktopAction,
  parseDesktopActionId,
  parseDesktopObservation,
  parseLearningCandidate,
  parseLearningCandidateId,
  parseMediaAsset,
  parseMediaAssetId,
  parseObservationId,
  parseRemoteDevice,
  parseRemoteDeviceId,
  parseWorkGraph,
  parseWorkGraphId,
  remoteDeviceSchema,
  resourceBudgetTelemetrySchema,
  signedEnvelopeSchema,
  workGraphSchema,
  type ArtifactPresentation,
  type ArtifactWorkbook,
  type CapabilityBundle,
  type CapabilityBundleId,
  type DesktopAction,
  type DesktopActionId,
  type DesktopObservation,
  type LearningCandidate,
  type LearningCandidateId,
  type MediaAsset,
  type MediaAssetId,
  type ObservationId,
  type P1EvidenceReference,
  type RemoteDevice,
  type RemoteDeviceId,
  type ResourceBudgetTelemetry,
  type SignedEnvelope,
  type WorkGraph,
  type WorkGraphId,
} from "../p1Contracts";

type P1ContractTypeWitness = [
  ArtifactWorkbook,
  ArtifactPresentation,
  DesktopObservation,
  DesktopAction,
  MediaAsset,
  RemoteDevice,
  CapabilityBundle,
  LearningCandidate,
  WorkGraph,
];

type P1SupportTypeWitness = [
  ObservationId,
  DesktopActionId,
  MediaAssetId,
  RemoteDeviceId,
  CapabilityBundleId,
  LearningCandidateId,
  WorkGraphId,
  P1EvidenceReference,
  SignedEnvelope,
  ResourceBudgetTelemetry,
];

type JsonObject = Record<string, unknown>;
type Parser = (value: unknown) => unknown;

const contracts = vectors.contracts as unknown as Record<string, JsonObject>;

const contractCases: readonly [string, Parser][] = [
  ["artifactWorkbook", parseArtifactWorkbook],
  ["artifactPresentation", parseArtifactPresentation],
  ["desktopObservation", parseDesktopObservation],
  ["desktopAction", parseDesktopAction],
  ["mediaAsset", parseMediaAsset],
  ["remoteDevice", parseRemoteDevice],
  ["capabilityBundle", parseCapabilityBundle],
  ["learningCandidate", parseLearningCandidate],
  ["workGraph", parseWorkGraph],
];

const contractSchemas = [
  artifactWorkbookSchema,
  artifactPresentationSchema,
  desktopObservationSchema,
  desktopActionSchema,
  mediaAssetSchema,
  remoteDeviceSchema,
  capabilityBundleSchema,
  learningCandidateSchema,
  workGraphSchema,
] as const;

function cloneFixture(name: string): JsonObject {
  return structuredClone(contracts[name]);
}

function nestedRecord(value: unknown, key: string): JsonObject {
  return (value as JsonObject)[key] as JsonObject;
}

function firstRecord(value: unknown, key: string): JsonObject {
  return ((value as JsonObject)[key] as JsonObject[])[0];
}

describe("P1 contract parity", () => {
  it("parses all nine shared vectors and exposes the frozen discriminants", () => {
    const typeWitness: P1ContractTypeWitness | null = null;
    const supportTypeWitness: P1SupportTypeWitness | null = null;
    expect(typeWitness).toBeNull();
    expect(supportTypeWitness).toBeNull();
    expect(P1_CONTRACT_VERSION).toBe(vectors.schemaVersion);

    const parsedTypes = contractCases.map(([name, parser]) => {
      const parsed = parser(cloneFixture(name)) as JsonObject;
      expect(parsed.schemaVersion).toBe(P1_CONTRACT_VERSION);
      return parsed.contractType;
    });

    expect(parsedTypes).toEqual(P1_CONTRACT_TYPES);
    expect(contractSchemas).toHaveLength(9);
    expect(DESKTOP_ACTION_KINDS).toContain("type");
    expect(MEDIA_KINDS).toContain("image");
    expect(REMOTE_DEVICE_SCOPES).toContain("stop_task");
    expect(CAPABILITY_KINDS).toContain("mutation");
    expect(WORK_GRAPH_NODE_KINDS).toContain("synthesis");

    expect(
      evidenceReferenceSchema.parse(firstRecord(contracts.artifactWorkbook, "evidence")),
    ).toBeDefined();
    expect(
      signedEnvelopeSchema.parse(nestedRecord(contracts.remoteDevice, "signature")),
    ).toBeDefined();
    expect(
      resourceBudgetTelemetrySchema.parse(
        nestedRecord(contracts.workGraph, "resourceBudget"),
      ),
    ).toBeDefined();
  });

  it("round-trips every new opaque identifier using the P0 representation", () => {
    const ids = vectors.idVectors;
    expect(parseObservationId(ids.observation)).toBe(ids.observation);
    expect(parseDesktopActionId(ids.action)).toBe(ids.action);
    expect(parseMediaAssetId(ids.mediaAsset)).toBe(ids.mediaAsset);
    expect(parseRemoteDeviceId(ids.remoteDevice)).toBe(ids.remoteDevice);
    expect(parseCapabilityBundleId(ids.capabilityBundle)).toBe(ids.capabilityBundle);
    expect(parseLearningCandidateId(ids.learningCandidate)).toBe(ids.learningCandidate);
    expect(parseWorkGraphId(ids.workGraph)).toBe(ids.workGraph);

    for (const parser of [
      parseObservationId,
      parseDesktopActionId,
      parseMediaAssetId,
      parseRemoteDeviceId,
      parseCapabilityBundleId,
      parseLearningCandidateId,
      parseWorkGraphId,
    ]) {
      expect(() => parser("observation_00000000-0000-0000-0000-000000000000")).toThrow();
    }
  });

  it.each(contractCases)("rejects zero and unknown %s versions", (name, parser) => {
    expect(() => parser({ ...cloneFixture(name), schemaVersion: 0 })).toThrow();
    expect(() => parser({ ...cloneFixture(name), schemaVersion: 2 })).toThrow();
  });

  it.each(contractCases)("rejects unknown %s evidence kinds", (name, parser) => {
    const payload = cloneFixture(name);
    firstRecord(payload, "evidence").evidenceClass = "claimed_success";
    expect(() => parser(payload)).toThrow();
  });

  it.each(contractCases)("preserves unknown optional %s fields", (name, parser) => {
    const payload = cloneFixture(name);
    payload.futureMetadata = { introducedIn: 2 };
    firstRecord(payload, "evidence").futureEvidenceDetail = "retained";

    const parsed = parser(payload) as JsonObject;
    expect(parsed.futureMetadata).toEqual({ introducedIn: 2 });
    expect(firstRecord(parsed, "evidence").futureEvidenceDetail).toBe("retained");
  });

  it("rejects cross-project references in every P1 envelope", () => {
    const crossProject = vectors.idVectors.crossProject;
    const mutations: readonly [string, Parser, (payload: JsonObject) => void][] = [
      ["artifactWorkbook", parseArtifactWorkbook, (payload) => {
        firstRecord(payload, "worksheets").projectId = crossProject;
      }],
      ["artifactPresentation", parseArtifactPresentation, (payload) => {
        firstRecord(payload, "slides").projectId = crossProject;
      }],
      ["desktopObservation", parseDesktopObservation, (payload) => {
        firstRecord(payload, "elements").projectId = crossProject;
      }],
      ["desktopAction", parseDesktopAction, (payload) => {
        nestedRecord(payload, "target").projectId = crossProject;
      }],
      ["mediaAsset", parseMediaAsset, (payload) => {
        nestedRecord(payload, "source").projectId = crossProject;
      }],
      ["remoteDevice", parseRemoteDevice, (payload) => {
        firstRecord(payload, "evidence").projectId = crossProject;
      }],
      ["capabilityBundle", parseCapabilityBundle, (payload) => {
        firstRecord(payload, "evidence").projectId = crossProject;
      }],
      ["learningCandidate", parseLearningCandidate, (payload) => {
        firstRecord(payload, "sourceTasks").projectId = crossProject;
      }],
      ["workGraph", parseWorkGraph, (payload) => {
        firstRecord(payload, "nodes").projectId = crossProject;
      }],
    ];

    for (const [name, parser, mutate] of mutations) {
      const payload = cloneFixture(name);
      mutate(payload);
      expect(() => parser(payload), name).toThrow("Cross-project references");
    }
  });

  it("rejects unsigned or structurally unsigned remote and package envelopes", () => {
    const remote = cloneFixture("remoteDevice");
    delete remote.signature;
    expect(() => parseRemoteDevice(remote)).toThrow();

    const bundle = cloneFixture("capabilityBundle");
    delete bundle.signature;
    expect(() => parseCapabilityBundle(bundle)).toThrow();

    const malformed = cloneFixture("capabilityBundle");
    nestedRecord(malformed, "signature").signature = "not-an-ed25519-signature";
    expect(() => parseCapabilityBundle(malformed)).toThrow();
  });

  it("requires the production approval grant representation", () => {
    const action = cloneFixture("desktopAction");
    action.approvalId = "approval_legacy";
    expect(() => parseDesktopAction(action)).toThrow();
  });

  it("freezes the minimum P1 authority and policy invariants", () => {
    const action = cloneFixture("desktopAction");
    delete action.expectedPostcondition;
    expect(() => parseDesktopAction(action)).toThrow();

    for (const policy of [
      "retentionPolicy",
      "redactionPolicy",
      "providerRoutingPolicy",
    ]) {
      const media = cloneFixture("mediaAsset");
      delete media[policy];
      expect(() => parseMediaAsset(media), policy).toThrow();
    }

    const bundle = cloneFixture("capabilityBundle");
    delete bundle.requestedGrants;
    expect(() => parseCapabilityBundle(bundle)).toThrow();

    const digestMismatch = cloneFixture("capabilityBundle");
    nestedRecord(digestMismatch, "signature").payloadSha256 = "0".repeat(64);
    expect(() => parseCapabilityBundle(digestMismatch)).toThrow(
      "signature digest does not match",
    );

    const candidate = cloneFixture("learningCandidate");
    delete candidate.proposedDiff;
    expect(() => parseLearningCandidate(candidate)).toThrow();
    candidate.proposedDiff = cloneFixture("learningCandidate").proposedDiff;
    candidate.candidateVersion = 0;
    expect(() => parseLearningCandidate(candidate)).toThrow();

    const graph = cloneFixture("workGraph");
    graph.parentOwnsMutations = false;
    expect(() => parseWorkGraph(graph)).toThrow();
  });

  it("requires bounded WorkGraph resource telemetry and declared dependencies", () => {
    const graph = cloneFixture("workGraph");
    expect(parseWorkGraph(graph).resourceBudget.usage.toolCalls).toBe(31);
    expect(parseWorkGraph(graph).resourceBudget.limits.concurrentChildren).toBe(8);
    expect(parseWorkGraph(graph).resourceBudget.limits.mutations).toBe(12);
    expect(parseWorkGraph(graph).resourceBudget.usage.peakConcurrentChildren).toBe(1);
    expect(parseWorkGraph(graph).resourceBudget.usage.mutationsCommitted).toBe(0);

    delete graph.resourceBudget;
    expect(() => parseWorkGraph(graph)).toThrow();

    const unknownDependency = cloneFixture("workGraph");
    firstRecord(unknownDependency, "nodes").dependsOn = ["missing"];
    expect(() => parseWorkGraph(unknownDependency)).toThrow(
      "dependencies must reference another declared node",
    );
  });

  it("rejects cyclic WorkGraph dependencies", () => {
    const graph = cloneFixture("workGraph");
    firstRecord(graph, "nodes").dependsOn = ["synthesis"];
    expect(() => parseWorkGraph(graph)).toThrow("dependency cycles are not allowed");
  });

  it("requires one WorkGraph concurrency ceiling", () => {
    const graph = cloneFixture("workGraph");
    graph.maxConcurrentChildren = 7;
    expect(() => parseWorkGraph(graph)).toThrow(
      "concurrency must match its resource budget limit",
    );
  });

  it.each([
    ["tokens", 120001],
    ["wallTimeMs", 3600001],
    ["peakMemoryBytes", 8589934593],
    ["processes", 17],
    ["networkRequests", 401],
    ["toolCalls", 1001],
    ["mutationAttempts", 13],
  ])("rejects WorkGraph %s usage above its limit", (field, value) => {
    const graph = cloneFixture("workGraph");
    const usage = nestedRecord(nestedRecord(graph, "resourceBudget"), "usage");
    usage[field] = value;
    expect(() => parseWorkGraph(graph)).toThrow(
      "resource usage exceeds its declared limit",
    );
  });

  it("rejects peak WorkGraph concurrency above the shared ceiling", () => {
    const graph = cloneFixture("workGraph");
    graph.maxConcurrentChildren = 7;
    const budget = nestedRecord(graph, "resourceBudget");
    nestedRecord(budget, "limits").concurrentChildren = 7;
    nestedRecord(budget, "usage").peakConcurrentChildren = 8;
    expect(() => parseWorkGraph(graph)).toThrow(
      "resource usage exceeds its declared limit",
    );
  });

  it("rejects committed WorkGraph mutations without corresponding attempts", () => {
    const graph = cloneFixture("workGraph");
    const usage = nestedRecord(nestedRecord(graph, "resourceBudget"), "usage");
    usage.mutationAttempts = 1;
    usage.mutationsCommitted = 2;
    expect(() => parseWorkGraph(graph)).toThrow(
      "Committed mutations cannot exceed mutation attempts",
    );
  });
});
