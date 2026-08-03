#!/usr/bin/env node

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign as signPayload,
} from "node:crypto";
import { chmodSync, lstatSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import {
  atomicWriteFile,
  TRUSTED_RELEASE_PUBLIC_KEY_HEX,
} from "./release-manifest.mjs";

export const EVIDENCE_SCHEMA_VERSION = 1;
export const EVIDENCE_KIND = "oomu.executed-release-evidence";

export const EVIDENCE_FRESHNESS_MS = Object.freeze({
  source_provenance: 24 * 60 * 60 * 1000,
  apple_toolchain: 24 * 60 * 60 * 1000,
  dependency_audit: 24 * 60 * 60 * 1000,
  pdf_containment: 24 * 60 * 60 * 1000,
  automated_tests: 24 * 60 * 60 * 1000,
  release_sanitizer: 24 * 60 * 60 * 1000,
  database_sanitizer: 24 * 60 * 60 * 1000,
  entitlement_snapshot: 24 * 60 * 60 * 1000,
  artifact_validation: 24 * 60 * 60 * 1000,
  signing: 24 * 60 * 60 * 1000,
  notarization: 24 * 60 * 60 * 1000,
  stapling: 24 * 60 * 60 * 1000,
  manifest_verification: 24 * 60 * 60 * 1000,
  golden_task_matrix: 24 * 60 * 60 * 1000,
  recovery_matrix: 24 * 60 * 60 * 1000,
  hero_workflow: 24 * 60 * 60 * 1000,
  privacy_declarations: 24 * 60 * 60 * 1000,
  clean_machine_launch: 7 * 24 * 60 * 60 * 1000,
});

export const REQUIRED_EVIDENCE_TYPES = Object.freeze(Object.keys(EVIDENCE_FRESHNESS_MS));

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function requireText(value, label) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string.`);
  }
}

function isoTimestamp(value, label) {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) throw new Error(`${label} is not a valid timestamp.`);
  return parsed;
}

export function createEvidenceRecord({
  evidenceType,
  buildIdentifier,
  sourceRevision,
  artifactIdentifier,
  artifactDigest,
  producer,
  execution,
  result,
  producedAt = new Date(),
  expiresAt,
}) {
  const freshness = EVIDENCE_FRESHNESS_MS[evidenceType];
  if (!freshness) throw new Error(`Unknown evidence type: ${evidenceType}`);
  if (!execution || execution.executed !== true || execution.exit_code !== 0) {
    throw new Error(`${evidenceType} evidence requires a successful executed command.`);
  }
  requireText(producer?.executable, "producer.executable");
  requireText(producer?.component, "producer.component");
  requireText(producer?.endpoint, "producer.endpoint");
  requireText(producer?.input, "producer.input");
  const produced = producedAt instanceof Date ? producedAt : new Date(producedAt);
  if (!Number.isFinite(produced.getTime())) throw new Error("Invalid evidence production time.");
  const maximumExpiration = produced.getTime() + freshness;
  const requestedExpiration = expiresAt === undefined
    ? maximumExpiration
    : isoTimestamp(expiresAt, "expiresAt");
  const expiration = Math.min(maximumExpiration, requestedExpiration);
  if (expiration <= produced.getTime()) {
    throw new Error(`${evidenceType} expiration must be later than its production time.`);
  }
  return {
    schema_version: EVIDENCE_SCHEMA_VERSION,
    kind: EVIDENCE_KIND,
    evidence_type: evidenceType,
    status: "passed",
    synthetic: false,
    build_identifier: buildIdentifier,
    source_revision: sourceRevision,
    artifact_identifier: artifactIdentifier,
    artifact_digest: artifactDigest,
    produced_at: produced.toISOString(),
    expires_at: new Date(expiration).toISOString(),
    producer,
    execution,
    result,
  };
}

export function writeEvidenceRecord(evidenceDir, record) {
  const path = join(resolve(evidenceDir), `${record.evidence_type}.json`);
  atomicWriteFile(path, `${JSON.stringify(record, null, 2)}\n`);
  return path;
}

function validateEvidenceRecord(record, expected, nowMs) {
  if (
    record?.schema_version !== EVIDENCE_SCHEMA_VERSION ||
    record?.kind !== EVIDENCE_KIND ||
    !EVIDENCE_FRESHNESS_MS[record?.evidence_type]
  ) {
    throw new Error("Unsupported release evidence record.");
  }
  if (record.status !== "passed") {
    throw new Error(`${record.evidence_type} did not pass.`);
  }
  if (record.synthetic !== false || record.execution?.executed !== true) {
    throw new Error(`${record.evidence_type} is synthetic or was not executed.`);
  }
  if (record.execution.exit_code !== 0) {
    throw new Error(`${record.evidence_type} command did not exit successfully.`);
  }
  requireText(record.producer?.executable, "producer.executable");
  requireText(record.producer?.component, "producer.component");
  requireText(record.producer?.endpoint, "producer.endpoint");
  requireText(record.producer?.input, "producer.input");
  const syntheticWords = /(?:^|[\s/_-])(mock|simulat(?:ed|ion)|synthetic)(?:$|[\s/_.-])/i;
  for (const [label, value] of Object.entries(record.producer)) {
    if (typeof value === "string" && syntheticWords.test(value)) {
      throw new Error(`${record.evidence_type} producer ${label} identifies a simulated source.`);
    }
  }
  if (record.build_identifier !== expected.buildIdentifier) {
    throw new Error(`${record.evidence_type} belongs to another build.`);
  }
  if (record.source_revision !== expected.sourceRevision) {
    throw new Error(`${record.evidence_type} belongs to another source revision.`);
  }
  if (record.artifact_identifier !== expected.artifactIdentifier) {
    throw new Error(`${record.evidence_type} belongs to another artifact.`);
  }
  if (record.artifact_digest !== expected.artifactDigest) {
    throw new Error(`${record.evidence_type} belongs to another artifact digest.`);
  }
  const produced = isoTimestamp(record.produced_at, "produced_at");
  const expires = isoTimestamp(record.expires_at, "expires_at");
  const maximumFreshness = EVIDENCE_FRESHNESS_MS[record.evidence_type];
  if (produced > nowMs + 60_000) throw new Error(`${record.evidence_type} is future-dated.`);
  if (expires <= nowMs) throw new Error(`${record.evidence_type} evidence is stale.`);
  if (expires <= produced || expires - produced > maximumFreshness) {
    throw new Error(`${record.evidence_type} has an invalid freshness interval.`);
  }
  return record;
}

export function validateEvidenceBundle({
  evidenceDir,
  buildIdentifier,
  sourceRevision,
  artifactIdentifier,
  artifactDigest,
  now = new Date(),
  requireImmutable = true,
}) {
  const directory = resolve(evidenceDir);
  const nowMs = now instanceof Date ? now.getTime() : new Date(now).getTime();
  if (!Number.isFinite(nowMs)) throw new Error("Invalid evidence validation time.");
  const expected = {
    buildIdentifier,
    sourceRevision,
    artifactIdentifier,
    artifactDigest,
  };
  const checks = [];
  let minimumEvidenceExpiration = Number.POSITIVE_INFINITY;
  for (const evidenceType of REQUIRED_EVIDENCE_TYPES) {
    const path = join(directory, `${evidenceType}.json`);
    const stats = lstatSync(path);
    if (!stats.isFile() || stats.isSymbolicLink()) {
      throw new Error(`${evidenceType} evidence must be a regular file.`);
    }
    if (requireImmutable && (stats.mode & 0o222) !== 0) {
      throw new Error(`${evidenceType} evidence is writable and therefore not immutable.`);
    }
    const bytes = readFileSync(path);
    const record = validateEvidenceRecord(JSON.parse(bytes.toString("utf8")), expected, nowMs);
    minimumEvidenceExpiration = Math.min(
      minimumEvidenceExpiration,
      Date.parse(record.expires_at),
    );
    if (record.evidence_type !== evidenceType) {
      throw new Error(`Evidence filename/type mismatch for ${basename(path)}.`);
    }
    checks.push({ evidence_type: evidenceType, sha256: sha256(bytes) });
  }
  return {
    schema_version: 1,
    kind: "oomu.release-evidence-gate",
    status: "passed",
    synthetic: false,
    strict_mlc_mode: true,
    build_identifier: buildIdentifier,
    source_revision: sourceRevision,
    artifact_identifier: artifactIdentifier,
    artifact_digest: artifactDigest,
    verified_at: new Date(nowMs).toISOString(),
    expires_at: new Date(
      Math.min(nowMs + 24 * 60 * 60 * 1000, minimumEvidenceExpiration),
    ).toISOString(),
    checks,
  };
}

export function finalizeEvidenceBundle(options) {
  const payload = validateEvidenceBundle(options);
  if (!options.privateKeyPath) throw new Error("A release gate private key is required.");
  const privateKey = createPrivateKey(readFileSync(resolve(options.privateKeyPath)));
  if (privateKey.asymmetricKeyType !== "ed25519") {
    throw new Error("Release evidence gates must be signed with an Ed25519 key.");
  }
  const publicKey = createPublicKey(privateKey);
  const rawPublicKey = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  if (rawPublicKey.toString("hex") !== TRUSTED_RELEASE_PUBLIC_KEY_HEX) {
    throw new Error("Release gate key does not match the reviewed release trust root.");
  }
  const payloadJson = JSON.stringify(payload);
  const payloadBytes = Buffer.from(payloadJson, "utf8");
  const gate = {
    schema_version: 1,
    kind: "oomu.signed-release-evidence-gate",
    payload_json: payloadJson,
    payload_sha256: sha256(payloadBytes),
    signature: {
      algorithm: "ed25519",
      public_key_hex: rawPublicKey.toString("hex"),
      key_fingerprint_sha256: sha256(rawPublicKey),
      value_base64: signPayload(null, payloadBytes, privateKey).toString("base64"),
    },
  };
  const gatePath = join(resolve(options.evidenceDir), "release-gate.json");
  atomicWriteFile(gatePath, `${JSON.stringify(gate, null, 2)}\n`);
  chmodSync(resolve(options.evidenceDir), 0o555);
  return gate;
}

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith("--")) throw new Error(`Unexpected argument: ${key}`);
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`${key} requires a value.`);
    values[key.slice(2)] = value;
    index += 1;
  }
  return values;
}

function required(values, key) {
  if (!values[key]) throw new Error(`--${key} is required.`);
  return values[key];
}

function main() {
  const values = parseArgs(process.argv.slice(2));
  const gate = finalizeEvidenceBundle({
    evidenceDir: required(values, "evidence-dir"),
    buildIdentifier: required(values, "build-id"),
    sourceRevision: required(values, "source-revision"),
    artifactIdentifier: required(values, "artifact-id"),
    artifactDigest: required(values, "artifact-digest"),
    privateKeyPath: required(values, "private-key"),
  });
  const payload = JSON.parse(gate.payload_json);
  console.log(`Release evidence gate passed with ${payload.checks.length} executed checks.`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU RELEASE EVIDENCE ERROR: ${error.message}`);
    process.exit(1);
  }
}
