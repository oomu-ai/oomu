#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { verifyReleaseManifest } from "./release-manifest.mjs";
import { stableJson } from "./release-provenance.mjs";
import { inspectSourceIdentity } from "./source-identity.mjs";

export const RELEASE_CANDIDATE_DESCRIPTOR_KIND = "oomu.signed-candidate-input";
export const RELEASE_CANDIDATE_DESCRIPTOR_SCHEMA_VERSION = 1;
export const RELEASE_CANDIDATE_BINDING_KIND = "oomu.canonical-release-entrypoint-binding";

const SHA256_PATTERN = /^[a-f0-9]{64}$/u;
const SOURCE_REVISION_PATTERN = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/u;

function requiredString(value, label) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${label} is required.`);
  }
  return value.trim();
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function validReleaseBinding(binding) {
  return binding?.kind === RELEASE_CANDIDATE_BINDING_KIND
    && binding.schemaVersion === 1
    && binding.entrypoint === "scripts/release.mjs"
    && typeof binding.releaseProvenancePath === "string"
    && [
      binding.releaseProvenanceSha256,
      binding.releaseProvenanceStableSha256,
      binding.manifestSha256,
      binding.manifestPayloadSha256,
    ].every((value) => SHA256_PATTERN.test(value ?? ""))
    && /^sha256:[a-f0-9]{64}$/u.test(binding.signedOutputDigest ?? "");
}

export function signedCandidateDescriptorValue(input, sourceIdentity, releaseBinding) {
  if (!sourceIdentity?.worktreeClean
    || !SOURCE_REVISION_PATTERN.test(sourceIdentity.sourceRevision ?? "")
    || !SHA256_PATTERN.test(sourceIdentity.sourceContentSha256 ?? "")
    || !SHA256_PATTERN.test(sourceIdentity.worktreeStatusSha256 ?? "")) {
    throw new Error("A signed candidate descriptor requires one clean, exact source identity.");
  }
  if (!validReleaseBinding(releaseBinding)) {
    throw new Error(
      "A signed candidate descriptor requires canonical cryptographic release provenance.",
    );
  }
  const buildNumber = Number(input.expectedBuildNumber);
  if (!Number.isSafeInteger(buildNumber) || buildNumber < 1) {
    throw new Error("The signed candidate build number is invalid.");
  }
  return {
    kind: RELEASE_CANDIDATE_DESCRIPTOR_KIND,
    schemaVersion: RELEASE_CANDIDATE_DESCRIPTOR_SCHEMA_VERSION,
    appPath: requiredString(input.appPath, "Application path"),
    containerPath: requiredString(input.containerPath, "Container path"),
    entitlementReportPath: requiredString(
      input.entitlementReportPath, "Entitlement report path"),
    manifestPath: requiredString(input.manifestPath, "Manifest path"),
    publicKeyPath: requiredString(input.publicKeyPath, "Manifest public key path"),
    appPrefix: requiredString(input.appPrefix, "Application manifest prefix"),
    expectedTeamId: requiredString(input.expectedTeamId, "Team ID"),
    expectedBundleIdentifier: requiredString(input.expectedBundleIdentifier, "Bundle ID"),
    expectedBuildNumber: buildNumber,
    expectedBuildIdentifier: requiredString(
      input.expectedBuildIdentifier, "Build identifier"),
    expectedSourceRevision: sourceIdentity.sourceRevision,
    expectedArtifactIdentifier: requiredString(
      input.expectedArtifactIdentifier, "Artifact identifier"),
    sourceContentSha256: sourceIdentity.sourceContentSha256,
    sourceWorktreeStatusSha256: sourceIdentity.worktreeStatusSha256,
    sourceWorktreeClean: true,
    canonicalRelease: releaseBinding,
  };
}

function exactExistingPath(path, label, kind) {
  const absolute = resolve(requiredString(path, label));
  if (absolute !== path || !existsSync(absolute) || lstatSync(absolute).isSymbolicLink()
    || realpathSync(absolute) !== absolute
    || (kind === "file" ? !lstatSync(absolute).isFile() : !lstatSync(absolute).isDirectory())) {
    throw new Error(`${label} must be an exact, real ${kind}.`);
  }
  return absolute;
}

export function inspectCanonicalReleaseBinding(input, sourceIdentity, options = {}) {
  const appPath = exactExistingPath(input.appPath, "Application path", "directory");
  const manifestPath = exactExistingPath(input.manifestPath, "Manifest", "file");
  const publicKeyPath = exactExistingPath(input.publicKeyPath, "Manifest public key", "file");
  const provenancePath = exactExistingPath(
    input.releaseProvenancePath, "Release provenance", "file");
  const provenanceBytes = readFileSync(provenancePath);
  const provenance = JSON.parse(provenanceBytes.toString("utf8"));
  const manifest = verifyReleaseManifest({
    treeRoot: dirname(appPath),
    manifestPath,
    publicKeyPath,
    expectedBuildIdentifier: requiredString(
      input.expectedBuildIdentifier, "Build identifier"),
    expectedSourceRevision: sourceIdentity.sourceRevision,
    expectedArtifactIdentifier: requiredString(
      input.expectedArtifactIdentifier, "Artifact identifier"),
    expectedReleaseProvenance: provenance,
    ...(options.trustedPublicKeyHex
      ? { trustedPublicKeyHex: options.trustedPublicKeyHex }
      : {}),
  });
  const appSubtree = manifest.subtrees?.find((entry) => entry.path_prefix === input.appPrefix);
  const expectedBuildNumber = Number(input.expectedBuildNumber);
  if (provenance?.schemaVersion !== 1 || provenance.kind !== "oomu.release-provenance"
    || provenance.workflowSourceCommit !== sourceIdentity.sourceRevision
    || provenance.signedOutputDigest !== manifest.artifact_digest
    || provenance.releaseCandidateIntegrity?.applicationTreeDigest
      !== appSubtree?.artifact_digest
    || Number(provenance.releaseCandidateIntegrity?.buildNumber) !== expectedBuildNumber
    || Number(provenance.releaseVersion?.buildNumber) !== expectedBuildNumber
    || stableJson(manifest.release_provenance) !== stableJson(provenance)) {
    throw new Error(
      "The signed candidate is not bound to canonical release provenance for these exact bytes.",
    );
  }
  return {
    kind: RELEASE_CANDIDATE_BINDING_KIND,
    schemaVersion: 1,
    entrypoint: "scripts/release.mjs",
    releaseProvenancePath: provenancePath,
    releaseProvenanceSha256: sha256(provenanceBytes),
    releaseProvenanceStableSha256: sha256(stableJson(provenance)),
    manifestSha256: sha256(readFileSync(manifestPath)),
    manifestPayloadSha256: manifest.payload_sha256,
    signedOutputDigest: manifest.artifact_digest,
  };
}

export function verifyBoundSignedCandidateDescriptor(value, options = {}) {
  if (value?.kind !== RELEASE_CANDIDATE_DESCRIPTOR_KIND
    || value.schemaVersion !== RELEASE_CANDIDATE_DESCRIPTOR_SCHEMA_VERSION
    || !SOURCE_REVISION_PATTERN.test(value.expectedSourceRevision ?? "")
    || !SHA256_PATTERN.test(value.sourceContentSha256 ?? "")
    || !SHA256_PATTERN.test(value.sourceWorktreeStatusSha256 ?? "")
    || value.sourceWorktreeClean !== true || !validReleaseBinding(value.canonicalRelease)) {
    throw new Error("A signed-candidate descriptor has an unsupported schema.");
  }
  const sourceIdentity = {
    sourceRevision: value.expectedSourceRevision,
    sourceContentSha256: value.sourceContentSha256,
    worktreeStatusSha256: value.sourceWorktreeStatusSha256,
    worktreeClean: true,
  };
  const binding = inspectCanonicalReleaseBinding({
    ...value,
    releaseProvenancePath: value.canonicalRelease.releaseProvenancePath,
  }, sourceIdentity, options);
  if (stableJson(binding) !== stableJson(value.canonicalRelease)) {
    throw new Error("The candidate descriptor is not bound to its exact canonical release.");
  }
  return value;
}

export function writeCanonicalSignedCandidateDescriptor({
  input, repositoryRoot, outputPath,
}) {
  const sourceIdentity = inspectSourceIdentity(repositoryRoot);
  const releaseBinding = inspectCanonicalReleaseBinding(input, sourceIdentity);
  const value = signedCandidateDescriptorValue(input, sourceIdentity, releaseBinding);
  const output = resolve(requiredString(outputPath, "Output path"));
  if (output !== outputPath || existsSync(output)
    || !existsSync(dirname(output)) || realpathSync(dirname(output)) !== dirname(output)) {
    throw new Error("The descriptor output must be a new file in an exact existing directory.");
  }
  writeFileSync(output, `${JSON.stringify(value, null, 2)}\n`, {
    encoding: "utf8", flag: "wx", mode: 0o400,
  });
  return { path: output, value };
}

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("Use named value arguments for the release candidate descriptor.");
    }
    values[name.slice(2)] = value;
  }
  return values;
}

function main() {
  const values = parseArgs(process.argv.slice(2));
  const repositoryRoot = exactExistingPath(
    resolve(values["repository-root"] ?? resolve(import.meta.dirname, "..")),
    "Repository root",
    "directory",
  );
  const input = {
    appPath: exactExistingPath(values.app, "Application path", "directory"),
    containerPath: exactExistingPath(values.container, "Container path", "file"),
    entitlementReportPath: exactExistingPath(values.entitlements, "Entitlement report", "file"),
    manifestPath: exactExistingPath(values.manifest, "Manifest", "file"),
    publicKeyPath: exactExistingPath(values["public-key"], "Manifest public key", "file"),
    appPrefix: values["app-prefix"],
    expectedTeamId: values["team-id"],
    expectedBundleIdentifier: values["bundle-id"],
    expectedBuildNumber: values["build-number"],
    expectedBuildIdentifier: values["build-identifier"],
    expectedArtifactIdentifier: values["artifact-id"],
    releaseProvenancePath: exactExistingPath(
      values.provenance, "Release provenance", "file"),
  };
  const output = resolve(requiredString(values.output, "Output path"));
  if (output !== values.output || output.startsWith(`${repositoryRoot}/`)) {
    throw new Error("The descriptor output must be a new file in an exact existing directory.");
  }
  writeCanonicalSignedCandidateDescriptor({ input, repositoryRoot, outputPath: output });
  process.stdout.write(`OOMU_SIGNED_CANDIDATE_DESCRIPTOR=${output}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
