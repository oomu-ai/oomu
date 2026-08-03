#!/usr/bin/env node

import { randomBytes } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import {
  artifactDigestForEntries,
  collectTreeEntries,
} from "./release-manifest.mjs";
import {
  loadReleasePolicy,
  runApproved,
  sha256Bytes,
  stableJson,
} from "./release-provenance.mjs";
import {
  loadReleaseVersionRecord,
  unsignedReleaseArchiveName,
  unsignedReleaseArtifactIdentity,
} from "./release-version.mjs";

export const UNSIGNED_HANDOFF_KIND = "oomu.unsigned-release-handoff";
export const UNSIGNED_HANDOFF_VERSION = 1;

function fullRevision(value) {
  if (!/^[0-9a-f]{40}$/iu.test(value ?? "")) {
    throw new Error("Unsigned handoff requires a full source revision.");
  }
  return value.toLowerCase();
}

function safeBuildIdentifier(value) {
  if (!/^[A-Za-z0-9._-]{8,128}$/u.test(value ?? "")) {
    throw new Error("Unsigned handoff build identifier is invalid.");
  }
  return value;
}

function exactLockDigests(root) {
  const files = [
    "package-lock.json",
    "src-tauri/Cargo.lock",
    "rust-toolchain.toml",
    "release/release-policy.json",
    "release/version.json",
  ];
  return Object.fromEntries(files.map((path) => [
    path,
    sha256Bytes(readFileSync(join(root, path))),
  ]));
}

function exactGatePolicyDigests(root) {
  const files = [
    "package.json",
    "scripts/check-rust-file-lines.sh",
    "scripts/release-version.mjs",
    "scripts/check-source-quality.mjs",
    "scripts/source-metric-baselines.json",
    "scripts/source-quality/source-metrics.mjs",
    "scripts/check-module-cycles.mjs",
    "scripts/module-cycle-baseline.json",
    "scripts/check-unused-exports.mjs",
    "scripts/unused-export-baseline.json",
    "scripts/check-repository-hygiene.mjs",
    "scripts/check-p1-contract-gate.mjs",
    "scripts/p1-contract-gate/artifact-fixture-helpers.mjs",
    "scripts/p1-contract-gate/microsoft-fixture.mjs",
  ];
  return Object.fromEntries(files.map((path) => [
    path,
    sha256Bytes(readFileSync(join(root, path))),
  ]));
}

export function createUnsignedHandoff({
  repositoryRoot,
  appPath,
  outputDirectory,
  buildIdentifier,
  sourceRevision,
  toolchain,
  gateResults,
}) {
  const root = realpathSync(resolve(repositoryRoot));
  const app = realpathSync(resolve(appPath));
  if (!statSync(app).isDirectory() || !app.endsWith(".app")) {
    throw new Error("Unsigned handoff must contain one real application bundle.");
  }
  const output = resolve(outputDirectory);
  if (existsSync(output)) throw new Error("Unsigned handoff output already exists.");
  mkdirSync(output, { recursive: false, mode: 0o700 });
  const releaseVersion = loadReleaseVersionRecord(root);
  const archiveName = unsignedReleaseArchiveName(
    releaseVersion,
    basename(app, ".app"),
  );
  const archivePath = join(output, archiveName);
  runApproved(toolchain, "ditto", [
    "-c", "-k", "--sequesterRsrc", "--keepParent", app, archivePath,
  ], { label: "archive unsigned application" });
  chmodSync(archivePath, 0o400);

  const entries = collectTreeEntries(app);
  const handoff = {
    schemaVersion: UNSIGNED_HANDOFF_VERSION,
    kind: UNSIGNED_HANDOFF_KIND,
    handoffId: randomBytes(24).toString("hex"),
    buildIdentifier: safeBuildIdentifier(buildIdentifier),
    sourceRevision: fullRevision(sourceRevision),
    policyId: toolchain.policy.policyId,
    policyDigest: toolchain.policyDigest,
    target: toolchain.policy.target,
    artifactIdentity: unsignedReleaseArtifactIdentity(
      releaseVersion,
      buildIdentifier,
    ),
    releaseVersion,
    archiveName,
    archiveSha256: sha256Bytes(readFileSync(archivePath)),
    unsignedTreeDigest: artifactDigestForEntries(entries),
    unsignedPayloadDigest: sha256Bytes(Buffer.from(stableJson(entries), "utf8")),
    unsignedEntryCount: entries.length,
    entries,
    lockDigests: exactLockDigests(root),
    gatePolicyDigests: exactGatePolicyDigests(root),
    actionCommits: toolchain.policy.actions,
    toolchain: {
      runner: toolchain.runner,
      tools: toolchain.tools,
      versions: toolchain.versions,
    },
    gates: gateResults,
    buildSignPhaseIsolated: true,
    createdAt: new Date().toISOString(),
  };
  const payload = `${JSON.stringify(handoff, null, 2)}\n`;
  const handoffPath = join(output, "unsigned-handoff.json");
  writeFileSync(handoffPath, payload, { mode: 0o400, flag: "wx" });
  const descriptor = {
    handoffPath,
    archivePath,
    handoffSha256: sha256Bytes(Buffer.from(payload, "utf8")),
    handoff,
  };
  writeFileSync(
    join(output, "unsigned-handoff.sha256"),
    `${descriptor.handoffSha256}  unsigned-handoff.json\n`,
    { mode: 0o400, flag: "wx" },
  );
  return descriptor;
}

function findSingleApp(directory) {
  const apps = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.endsWith(".app"));
  if (apps.length !== 1) throw new Error("Unsigned handoff must expand to exactly one app.");
  return realpathSync(join(directory, apps[0].name));
}

export function verifyAndExpandUnsignedHandoff({
  handoffPath,
  archivePath,
  expectedHandoffSha256,
  expectedBuildIdentifier,
  expectedSourceRevision,
  toolchain,
}) {
  const handoffBytes = readFileSync(resolve(handoffPath));
  if (sha256Bytes(handoffBytes) !== expectedHandoffSha256) {
    throw new Error("Unsigned handoff descriptor digest does not match the build job output.");
  }
  const handoff = JSON.parse(handoffBytes.toString("utf8"));
  const { policy, policyDigest } = loadReleasePolicy();
  const root = realpathSync(resolve(import.meta.dirname, ".."));
  const releaseVersion = loadReleaseVersionRecord(root);
  const expectedArtifactIdentity = unsignedReleaseArtifactIdentity(
    releaseVersion,
    expectedBuildIdentifier,
  );
  const gateNames = new Set((handoff.gates ?? []).map((gate) => gate.label));
  const requiredGates = [
    "lint", "version", "source-size", "real-components", "p0-architecture", "p1-contracts", "novice-ui",
    "module-cycles", "unused-exports", "repository-hygiene", "release-integrity",
    "i18n", "typecheck", "frontend", "dependency-audit",
    "rust-dependency-audit", "entitlement-snapshot",
    "pdf-containment", "cargo-check", "cargo-test", "compile",
    "unsigned-bundle", "unsigned-sanitizer", "unsigned-database-sanitizer",
    "asset-catalog-validation",
  ];
  if (
    handoff.schemaVersion !== UNSIGNED_HANDOFF_VERSION ||
    handoff.kind !== UNSIGNED_HANDOFF_KIND ||
    handoff.buildIdentifier !== expectedBuildIdentifier ||
    handoff.sourceRevision !== fullRevision(expectedSourceRevision) ||
    handoff.policyId !== policy.policyId ||
    handoff.policyDigest !== policyDigest ||
    handoff.target !== policy.target ||
    handoff.artifactIdentity !== expectedArtifactIdentity ||
    stableJson(handoff.releaseVersion) !== stableJson(releaseVersion) ||
    handoff.unsignedEntryCount !== handoff.entries?.length ||
    handoff.buildSignPhaseIsolated !== true ||
    (handoff.gates ?? []).some((gate) => gate.exitCode !== 0) ||
    requiredGates.some((gate) => !gateNames.has(gate)) ||
    stableJson(handoff.toolchain?.runner) !== stableJson(policy.protectedRunner) ||
    stableJson(handoff.toolchain?.tools) !== stableJson(toolchain.tools) ||
    stableJson(handoff.toolchain?.versions) !== stableJson(toolchain.versions) ||
    stableJson(handoff.actionCommits) !== stableJson(policy.actions) ||
    stableJson(handoff.lockDigests) !== stableJson(exactLockDigests(root)) ||
    stableJson(handoff.gatePolicyDigests) !== stableJson(exactGatePolicyDigests(root))
  ) {
    throw new Error("Unsigned handoff identity or immutable policy binding is invalid.");
  }
  const archive = resolve(archivePath);
  if (basename(archive) !== handoff.archiveName) {
    throw new Error("Unsigned handoff archive identity is invalid.");
  }
  if (sha256Bytes(readFileSync(archive)) !== handoff.archiveSha256) {
    throw new Error("Unsigned handoff archive was mutated before signing.");
  }
  const expansionRoot = mkdtempSync(join(tmpdir(), "oomu-protected-handoff-"));
  runApproved(toolchain, "ditto", ["-x", "-k", archive, expansionRoot], {
    label: "expand verified unsigned application",
  });
  const appPath = findSingleApp(expansionRoot);
  const entries = collectTreeEntries(appPath);
  if (
    artifactDigestForEntries(entries) !== handoff.unsignedTreeDigest ||
    sha256Bytes(Buffer.from(stableJson(entries), "utf8")) !== handoff.unsignedPayloadDigest ||
    stableJson(entries) !== stableJson(handoff.entries)
  ) {
    throw new Error("Expanded unsigned application differs from its deterministic tree manifest.");
  }
  return { handoff, appPath, expansionRoot };
}
