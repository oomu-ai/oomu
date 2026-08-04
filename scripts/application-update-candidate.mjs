import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import {
  verifyBoundSignedCandidateDescriptor,
} from "./release-candidate-descriptor.mjs";
import {
  artifactDigestForEntries,
  collectTreeEntries,
} from "./release-manifest.mjs";
import { inspectSourceIdentity } from "./source-identity.mjs";

const repositoryRoot = resolve(import.meta.dirname, "..");

function exactPath(value, label, kind) {
  const path = resolve(String(value ?? ""));
  if (
    !value
    || path !== value
    || !existsSync(path)
    || lstatSync(path).isSymbolicLink()
    || realpathSync(path) !== path
    || (kind === "file" ? !lstatSync(path).isFile() : !lstatSync(path).isDirectory())
  ) {
    throw new Error(`${label} must be an exact, real ${kind}.`);
  }
  return path;
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function assertDescriptorSourceIdentity(
  descriptorValue,
  currentSource,
  expectedSourceRevision,
) {
  if (
    !currentSource?.worktreeClean
    || currentSource.sourceRevision !== expectedSourceRevision
    || currentSource.sourceContentSha256 !== descriptorValue?.sourceContentSha256
    || currentSource.worktreeStatusSha256 !== descriptorValue?.sourceWorktreeStatusSha256
  ) {
    throw new Error("The signed candidate descriptor does not match the current clean source tree.");
  }
  return currentSource;
}

export function loadVerifiedApplicationUpdateCandidate({
  descriptorPath,
  record,
  sourceRevision,
  appPath = null,
  dmgPath = null,
}) {
  const descriptorFile = exactPath(descriptorPath, "Signed candidate descriptor", "file");
  const descriptorBytes = readFileSync(descriptorFile);
  const descriptorValue = JSON.parse(descriptorBytes.toString("utf8"));
  const currentSource = inspectSourceIdentity(repositoryRoot);
  assertDescriptorSourceIdentity(descriptorValue, currentSource, sourceRevision);
  const descriptor = verifyBoundSignedCandidateDescriptor(descriptorValue);
  const app = exactPath(descriptor.appPath, "Descriptor application", "directory");
  const dmg = exactPath(descriptor.containerPath, "Descriptor DMG", "file");
  if (
    descriptor.expectedSourceRevision !== sourceRevision
    || Number(descriptor.expectedBuildNumber) !== record.buildNumber
  ) {
    throw new Error("The signed candidate descriptor does not match the release authority.");
  }
  const provenancePath = exactPath(
    descriptor.canonicalRelease.releaseProvenancePath,
    "Release provenance",
    "file",
  );
  const provenance = JSON.parse(readFileSync(provenancePath, "utf8"));
  if (
    provenance?.workflowSourceCommit !== sourceRevision
    || provenance?.releaseVersion?.productVersion !== record.productVersion
    || provenance?.releaseVersion?.buildNumber !== record.buildNumber
    || provenance?.releaseVersion?.intendedTag !== record.intendedTag
    || provenance?.releaseVersion?.channel !== record.channel
    || provenance?.releaseVersion?.publicLabel !== record.publicLabel
    || provenance?.runnerIdentity?.architecture !== "arm64"
    || provenance?.releaseCandidateIntegrity?.teamId
      !== descriptor.canonicalRelease.teamId
    || provenance?.releaseCandidateIntegrity?.bundleIdentifier
      !== descriptor.canonicalRelease.bundleIdentifier
  ) {
    throw new Error("Canonical release provenance does not match release/version.json.");
  }
  if (appPath && exactPath(appPath, "Qualified application", "directory") !== app) {
    throw new Error("OOMU_QUALIFIED_APP_PATH is not the descriptor-bound application.");
  }
  if (dmgPath && exactPath(dmgPath, "Qualified DMG", "file") !== dmg) {
    throw new Error("OOMU_RELEASE_DMG_PATH is not the descriptor-bound DMG.");
  }
  return {
    descriptor,
    descriptorPath: descriptorFile,
    descriptorSha256: createHash("sha256").update(descriptorBytes).digest("hex"),
    sourceRevision,
    appPath: app,
    appTreeDigest: artifactDigestForEntries(collectTreeEntries(app)),
    dmgPath: dmg,
    dmgSha256: sha256File(dmg),
    updaterTarget: "darwin-aarch64",
  };
}
