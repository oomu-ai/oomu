#!/usr/bin/env node

import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import process from "node:process";
import {
  artifactDigestForEntries,
  collectTreeEntries,
} from "./release-manifest.mjs";
import { releaseArtifactIdentifier } from "./release-version.mjs";
import { runApproved, sha256Bytes } from "./release-provenance.mjs";
import { validateInstalledIntegrityEvidence } from "./release-clean-machine.mjs";

const root = resolve(import.meta.dirname, "..");
const releaseRunners = join(root, "scripts", "release-runners");

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(
      `not executed - environment not ready: ${name} is required for protected release qualification`,
    );
  }
  return value;
}

function readJson(path, label) {
  const value = JSON.parse(readFileSync(path, "utf8"));
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is not a JSON object.`);
  }
  return value;
}

function sha256File(path) {
  return sha256Bytes(readFileSync(path));
}

function baseReleaseEnvironment(context, additions = {}) {
  return {
    OOMU_BUILD_ID: context.buildIdentifier,
    OOMU_SOURCE_REVISION: context.sourceRevision,
    ...additions,
  };
}

function releaseLabEnvironment(context) {
  return baseReleaseEnvironment(context, {
    OOMU_RELEASE_LAB_URL: requiredEnvironment("OOMU_RELEASE_LAB_URL"),
    OOMU_RELEASE_LAB_CLIENT_CERT_PATH: requiredEnvironment(
      "OOMU_RELEASE_LAB_CLIENT_CERT_PATH",
    ),
    OOMU_RELEASE_LAB_CLIENT_KEY_PATH: requiredEnvironment(
      "OOMU_RELEASE_LAB_CLIENT_KEY_PATH",
    ),
    OOMU_RELEASE_LAB_CA_PATH: requiredEnvironment("OOMU_RELEASE_LAB_CA_PATH"),
  });
}

function runNode(context, label, args, environment = {}) {
  return runApproved(context.toolchain, "node", args, { label, environment });
}

function evidenceIdentity(path) {
  return {
    fileName: basename(path),
    sha256: sha256File(path),
    sizeBytes: statSync(path).size,
  };
}

function writeHelperIntegrityManifest(context) {
  const helpers = {};
  for (const name of ["artifact_build_helper", "oomu-artifact-pdf-helper"]) {
    const path = join(context.appPath, "Contents", "MacOS", name);
    if (!existsSync(path) || !statSync(path).isFile()) {
      throw new Error(`Required signed artifact helper is missing: ${name}.`);
    }
    helpers[name] = sha256File(path);
  }
  const destination = join(
    context.appPath,
    "Contents",
    "Resources",
    "oomu-helper-integrity.json",
  );
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(
    destination,
    `${JSON.stringify({ schemaVersion: 1, helpers }, null, 2)}\n`,
    { mode: 0o644 },
  );
  return destination;
}

export function prepareProtectedApplicationQualification() {
  return {
    evidenceDirectory: mkdtempSync(join(tmpdir(), "oomu-protected-app-evidence-")),
  };
}

export function finalizeProtectedApplicationQualification(context, prepared) {
  const helperIntegrityPath = writeHelperIntegrityManifest(context);
  return {
    authorizedResourcePaths: [helperIntegrityPath],
    evidence: {
      files: { helperIntegrity: evidenceIdentity(helperIntegrityPath) },
    },
    helperIntegrityPath,
    evidenceDirectory: prepared.evidenceDirectory,
  };
}

function runExtensionGates(context, evidenceDirectory) {
  const toolchainPath = join(evidenceDirectory, "protected-toolchain.json");
  const outputPath = join(evidenceDirectory, "release-extension-gates.json");
  writeFileSync(toolchainPath, `${JSON.stringify(context.toolchain, null, 2)}\n`, {
    mode: 0o600,
  });
  runNode(context, "run post-sign release extension gates", [
    "scripts/run-release-extension-gates.mjs",
    "--app", context.appPath,
    "--evidence-dir", evidenceDirectory,
    "--toolchain", toolchainPath,
    "--output", outputPath,
  ], baseReleaseEnvironment(context, {
    OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64: requiredEnvironment(
      "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
    ),
  }));
  const report = readJson(outputPath, "Post-sign release extension gates");
  if (report.status !== "passed" || report.synthetic !== false) {
    throw new Error("Post-sign release extension gates did not produce real passing evidence.");
  }
  return report;
}

export function qualifySignedApplication(context, prepared) {
  const originalTreeDigest = artifactDigestForEntries(collectTreeEntries(context.appPath));
  const extensionGates = runExtensionGates(context, prepared.evidenceDirectory);
  if (artifactDigestForEntries(collectTreeEntries(context.appPath)) !== originalTreeDigest) {
    throw new Error("Protected application qualification changed the signed application.");
  }
  return { extensionGates };
}

function assertCleanMachineEvidence(report, expected) {
  const requiredTargets = new Set(["macos-14-latest", "macos-15-latest", "macos-current"]);
  const requiredProbes = [
    "voice_capture",
    "vision_ocr",
    "pdf_build_render",
    "local_inference",
    "pdf_extraction",
  ];
  for (const row of report.os_matrix ?? []) {
    const passing = new Set(
      (row.helper_probes ?? []).filter((probe) => probe.passed === true)
        .map((probe) => probe.name),
    );
    if (
      !requiredTargets.delete(row.target) ||
      row.passed !== true ||
      row.synthetic !== false ||
      requiredProbes.some((name) => !passing.has(name))
    ) {
      throw new Error("Clean-machine release matrix is incomplete or not genuine.");
    }
  }
  const completedAt = Date.parse(report.completed_at);
  validateInstalledIntegrityEvidence(report, {
    manifestPayloadSha256: expected.manifestPayloadSha256,
    installedAppSubtree: expected.installedAppSubtree,
    expectedBuildNumber: expected.buildNumber,
    expectedBundleIdentifier: expected.bundleIdentifier,
    expectedManifestSha256: expected.manifestSha256,
    expectedAppTreeSha256: expected.appTreeSha256,
    expectedTeamId: expected.teamId,
    expectedDesignatedRequirementSha256: expected.designatedRequirementSha256,
  });
  if (
    report.status !== "passed" ||
    report.synthetic !== false ||
    report.build_identifier !== expected.buildIdentifier ||
    report.source_revision !== expected.sourceRevision ||
    report.artifact_identifier !== expected.artifactIdentifier ||
    report.artifact_digest !== expected.artifactDigest ||
    report.artifact_file_sha256 !== expected.dmgSha256 ||
    basename(report.launched_executable ?? "") !== expected.mainBinaryName ||
    report.launch_exit_code !== 0 ||
    report.installed_from_dmg !== true ||
    report.codesign_verified !== true ||
    report.stapled_ticket_verified !== true ||
    report.repository_present !== false ||
    report.repository_scripts_on_path !== false ||
    !report.machine_identifier ||
    !report.endpoint ||
    !report.installed_application ||
    requiredTargets.size !== 0 ||
    !Number.isFinite(completedAt) ||
    completedAt > Date.now() + 60_000 ||
    completedAt < Date.now() - 24 * 60 * 60 * 1000
  ) {
    throw new Error(
      "release_lab_integrity_evidence_incomplete: clean-machine runner did not attest this exact signed candidate",
    );
  }
}

function runCleanMachine(context, staged, evidenceDirectory, expected) {
  const outputPath = join(evidenceDirectory, "clean-machine-launch.json");
  runNode(context, "qualify exact DMG on clean release machines", [
    join(releaseRunners, "clean-machine-launch.mjs"),
    "--artifact", staged.stagedDmg,
    "--build-id", context.buildIdentifier,
    "--source-revision", context.sourceRevision,
    "--artifact-id", expected.artifactIdentifier,
    "--artifact-digest", expected.artifactDigest,
    "--artifact-file-sha256", expected.dmgSha256,
    "--manifest-payload-sha256", expected.manifestPayloadSha256,
    "--installed-app-prefix", expected.installedAppSubtree.path_prefix,
    "--installed-app-subtree-digest", expected.installedAppSubtree.artifact_digest,
    "--installed-app-entry-count", String(expected.installedAppSubtree.entry_count),
    "--manifest-sha256", expected.manifestSha256,
    "--app-tree-sha256", expected.appTreeSha256,
    "--bundle-identifier", expected.bundleIdentifier,
    "--build-number", String(expected.buildNumber),
    "--team-id", expected.teamId,
    "--designated-requirement-sha256", expected.designatedRequirementSha256,
    "--output", outputPath,
  ], releaseLabEnvironment(context));
  const report = readJson(outputPath, "Clean-machine release report");
  assertCleanMachineEvidence(report, expected);
  return { outputPath, report };
}

function runP0Acceptance(context, staged, evidenceDirectory, expected) {
  const outputDirectory = join(evidenceDirectory, "p0-acceptance");
  mkdirSync(outputDirectory, { recursive: false, mode: 0o700 });
  runNode(context, "execute P0 acceptance on exact signed DMG", [
    join(releaseRunners, "p0-acceptance.mjs"),
    "--artifact", staged.stagedDmg,
    "--build-id", context.buildIdentifier,
    "--source-revision", context.sourceRevision,
    "--artifact-id", expected.artifactIdentifier,
    "--artifact-digest", expected.artifactDigest,
    "--output-dir", outputDirectory,
  ], releaseLabEnvironment(context));
  const validation = runNode(context, "validate exact P0 acceptance evidence", [
    "scripts/p0-release-acceptance.mjs",
    `--evidence-dir=${outputDirectory}`,
    `--build-id=${context.buildIdentifier}`,
    `--source-revision=${context.sourceRevision}`,
    `--artifact-digest=${expected.artifactDigest}`,
  ]);
  const result = JSON.parse(validation.stdout);
  if (result.status !== "passed" || result.synthetic !== false) {
    throw new Error("P0 release acceptance did not produce real passing evidence.");
  }
  const files = readdirSync(outputDirectory).sort().map((name) => {
    const path = join(outputDirectory, name);
    return { ...evidenceIdentity(path), name };
  });
  return { files, result };
}

export function qualifySignedCandidate(context, staged, manifestPath, candidateIntegrity) {
  const evidenceDirectory = join(context.outputDirectory, "qualification");
  mkdirSync(evidenceDirectory, { recursive: false, mode: 0o700 });
  const beforeDmg = sha256File(staged.stagedDmg);
  const beforeTree = artifactDigestForEntries(collectTreeEntries(staged.candidateDirectory));
  const manifest = readJson(manifestPath, "Signed release manifest");
  const installedAppSubtree = manifest.subtrees?.find(
    (subtree) => subtree.path_prefix === basename(staged.stagedApp),
  );
  if (!installedAppSubtree) {
    throw new Error(
      "release_manifest_app_subtree_missing: signed manifest has no application subtree",
    );
  }
  const mainIdentity = candidateIntegrity.codeObjects?.find((entry) => entry.path === ".");
  if (!mainIdentity?.designatedRequirementSha256) {
    throw new Error(
      "release_candidate_identity_incomplete: the verified candidate has no main designated requirement",
    );
  }
  const expected = {
    artifactIdentifier: releaseArtifactIdentifier(
      context.releaseVersion,
      context.buildIdentifier,
    ),
    artifactDigest: staged.signedOutputDigest,
    buildIdentifier: context.buildIdentifier,
    dmgSha256: beforeDmg,
    mainBinaryName: context.mainBinaryName,
    sourceRevision: context.sourceRevision,
    manifestPayloadSha256: manifest.payload_sha256,
    installedAppSubtree,
    buildNumber: context.releaseVersion.buildNumber,
    bundleIdentifier: JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    ).identifier,
    manifestSha256: sha256File(manifestPath),
    appTreeSha256: candidateIntegrity.application.treeDigest.replace(/^sha256:/u, ""),
    teamId: candidateIntegrity.application.teamId,
    designatedRequirementSha256: mainIdentity.designatedRequirementSha256,
  };
  const cleanMachine = runCleanMachine(context, staged, evidenceDirectory, expected);
  const p0Acceptance = runP0Acceptance(context, staged, evidenceDirectory, expected);
  if (
    sha256File(staged.stagedDmg) !== beforeDmg ||
    artifactDigestForEntries(collectTreeEntries(staged.candidateDirectory)) !== beforeTree
  ) {
    throw new Error("External protected qualification changed the signed candidate.");
  }
  chmodSync(staged.stagedDmg, 0o400);
  return {
    cleanMachine: cleanMachine.report,
    evidenceFiles: {
      cleanMachine: evidenceIdentity(cleanMachine.outputPath),
      p0Acceptance: p0Acceptance.files,
    },
    p0Acceptance: p0Acceptance.result,
  };
}
