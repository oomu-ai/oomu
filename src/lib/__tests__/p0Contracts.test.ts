import { describe, expect, it } from "vitest";
import vectors from "../../../schemas/p0-contract-v1-vectors.json";
import {
  EVIDENCE_CLASSES,
  P0_CONTRACT_VERSION,
  TASK_STATES,
  parseArtifactId,
  parseChildRunId,
  parseConnectorId,
  parseEvidenceClass,
  parseP0EventEnvelope,
  parseProjectId,
  parseTaskId,
  parseTaskRunId,
  parseTaskState,
  type ArtifactId,
  type ChildRunId,
  type ConnectorId,
  type EvidenceClass,
  type P0EventEnvelope,
  type ProjectId,
  type TaskId,
  type TaskRunId,
  type TaskState,
} from "../p0Contracts";

type P0ContractTypeWitness = [
  ProjectId,
  TaskId,
  TaskRunId,
  ArtifactId,
  ConnectorId,
  ChildRunId,
  TaskState,
  EvidenceClass,
  P0EventEnvelope,
];

const idParsers = [
  parseProjectId,
  parseTaskId,
  parseTaskRunId,
  parseArtifactId,
  parseConnectorId,
  parseChildRunId,
];

describe("P0 contract parity", () => {
  it("round-trips the shared identifier, state, evidence, and event vectors", () => {
    const typeWitness: P0ContractTypeWitness | null = null;
    expect(typeWitness).toBeNull();
    expect(P0_CONTRACT_VERSION).toBe(vectors.schemaVersion);
    expect(TASK_STATES).toEqual(vectors.taskStates);
    expect(EVIDENCE_CLASSES).toEqual(vectors.evidenceClasses);
    expect(parseProjectId(vectors.idVectors.project)).toBe(vectors.idVectors.project);
    expect(parseTaskId(vectors.idVectors.task)).toBe(vectors.idVectors.task);
    expect(parseTaskRunId(vectors.idVectors.taskRun)).toBe(vectors.idVectors.taskRun);
    expect(parseArtifactId(vectors.idVectors.artifact)).toBe(vectors.idVectors.artifact);
    expect(parseConnectorId(vectors.idVectors.connector)).toBe(vectors.idVectors.connector);
    expect(parseChildRunId(vectors.idVectors.childRun)).toBe(vectors.idVectors.childRun);
    for (const state of vectors.taskStates) expect(parseTaskState(state)).toBe(state);
    for (const evidence of vectors.evidenceClasses) expect(parseEvidenceClass(evidence)).toBe(evidence);
    expect(parseP0EventEnvelope(vectors.eventEnvelope)).toEqual(vectors.eventEnvelope);
  });

  it("fails closed for malformed identifiers and unknown vocabulary", () => {
    for (const invalid of vectors.invalidIds) {
      expect(idParsers.every((parser) => {
        try {
          parser(invalid);
          return false;
        } catch {
          return true;
        }
      })).toBe(true);
    }
    expect(() => parseTaskState("paused")).toThrow("Unknown P0 task state");
    expect(() => parseEvidenceClass("claimed_success")).toThrow("Unknown P0 evidence class");
  });

  it("rejects unknown fields, malformed versions, and non-UTC timestamps", () => {
    expect(() => parseP0EventEnvelope({ ...vectors.eventEnvelope, schemaVersion: 2 })).toThrow(
      "Unsupported P0 event envelope version",
    );
    expect(() => parseP0EventEnvelope({ ...vectors.eventEnvelope, timestamp: "2026-07-10 20:15:30" })).toThrow(
      "Invalid P0 event timestamp",
    );
    expect(() => parseP0EventEnvelope({ ...vectors.eventEnvelope, untrusted: true })).toThrow(
      "Unknown P0 event envelope field",
    );
  });
});
