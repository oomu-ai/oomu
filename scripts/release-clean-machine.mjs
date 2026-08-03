import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  readFileSync,
} from "node:fs";
import { basename, join } from "node:path";

export function validateInstalledIntegrityEvidence(report, expected) {
  const runtimeIdentity = report.runtime_identity_receipt;
  const singleInstance = report.single_instance_receipt;
  const cleanup = report.process_cleanup_receipt;
  const candidate = report.release_candidate_evidence;
  if (
    report.installed_application_manifest_payload_sha256 !== expected.manifestPayloadSha256 ||
    report.installed_application_subtree_digest !== expected.installedAppSubtree.artifact_digest ||
    report.installed_application_entry_count !== expected.installedAppSubtree.entry_count ||
    report.nested_code_objects_verified !== true ||
    report.semantic_entitlements_verified !== true ||
    candidate?.bundleIdentifier !== expected.expectedBundleIdentifier ||
    candidate?.channel !== "production" ||
    candidate?.buildNumber !== expected.expectedBuildNumber ||
    candidate?.appTreeSha256 !== expected.expectedAppTreeSha256 ||
    candidate?.manifestSha256 !== expected.expectedManifestSha256 ||
    candidate?.codesignVerified !== true ||
    candidate?.gatekeeperAccepted !== true ||
    candidate?.notarizationAccepted !== true ||
    candidate?.nestedExecutablesVerified !== true ||
    candidate?.installedTreeMatches !== true ||
    candidate?.teamId !== expected.expectedTeamId ||
    candidate?.designatedRequirementSha256 !==
      expected.expectedDesignatedRequirementSha256 ||
    candidate?.beforeQualificationSha256 !== expected.expectedAppTreeSha256 ||
    candidate?.afterQualificationSha256 !== expected.expectedAppTreeSha256 ||
    runtimeIdentity?.kind !== "runtime_identity" ||
    runtimeIdentity?.schemaVersion !== 1 ||
    runtimeIdentity?.status !== "verified" ||
    runtimeIdentity?.channel !== "production" ||
    runtimeIdentity?.profileClass !== "production" ||
    runtimeIdentity?.bundleIdentifier !== expected.expectedBundleIdentifier ||
    runtimeIdentity?.teamId !== expected.expectedTeamId ||
    runtimeIdentity?.buildNumber !== expected.expectedBuildNumber ||
    runtimeIdentity?.applicationDataNamespace !== expected.expectedBundleIdentifier ||
    runtimeIdentity?.keychainNamespaceClass !== "production" ||
    !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/u.test(runtimeIdentity?.codeDirectoryHash ?? "") ||
    !/^[a-f0-9]{64}$/u.test(runtimeIdentity?.executableSha256 ?? "") ||
    runtimeIdentity?.strictSignatureValid !== true ||
    runtimeIdentity?.designatedRequirementSha256 !==
      expected.expectedDesignatedRequirementSha256 ||
    typeof runtimeIdentity?.singleInstanceNamespace !== "string" ||
    runtimeIdentity.singleInstanceNamespace.length < 12 ||
    singleInstance?.kind !== "single_instance" ||
    singleInstance?.schemaVersion !== 2 ||
    singleInstance?.decision !== "primary_acquired" ||
    singleInstance?.namespace !== runtimeIdentity.singleInstanceNamespace ||
    singleInstance?.releaseChannel !== runtimeIdentity.channel ||
    singleInstance?.profileClass !== runtimeIdentity.profileClass ||
    singleInstance?.buildNumber !== runtimeIdentity.buildNumber ||
    singleInstance?.codeDirectoryHash !== runtimeIdentity.codeDirectoryHash ||
    singleInstance?.strictSignatureValid !== true ||
    !Number.isSafeInteger(singleInstance?.holderPid) ||
    singleInstance.holderPid <= 1 ||
    cleanup?.kind !== "exact_process_cleanup" ||
    cleanup?.schemaVersion !== 1 ||
    cleanup?.status !== "passed" ||
    cleanup?.synthetic !== false ||
    cleanup?.exitVerified !== true ||
    !Number.isSafeInteger(cleanup?.pid) ||
    cleanup.pid <= 1 ||
    !Number.isSafeInteger(cleanup?.parentPid) ||
    cleanup.parentPid < 1 ||
    !/^[a-f0-9]{64}$/u.test(cleanup?.executableIdentitySha256 ?? "") ||
    !/^[a-f0-9]{64}$/u.test(cleanup?.executableSha256 ?? "") ||
    !["already_stopped", "graceful", "forced"].includes(cleanup?.outcome) ||
    cleanup?.forced !== (cleanup?.outcome === "forced") ||
    !Number.isFinite(Date.parse(cleanup?.startedAt)) ||
    !Number.isFinite(Date.parse(cleanup?.completedAt))
  ) {
    throw new Error(
      "release_lab_integrity_evidence_incomplete: installed release evidence is incomplete",
    );
  }
  return true;
}

function validateCleanMachineReport({
  cleanMachine,
  buildId,
  sourceRevision,
  artifactId,
  artifactDigest,
  dmgSha256,
  mainBinaryName,
  expectedBuildNumber,
  expectedBundleIdentifier,
  manifestPayloadSha256,
  installedAppSubtree,
  expectedManifestSha256,
  expectedAppTreeSha256,
  expectedTeamId,
  expectedDesignatedRequirementSha256,
}) {
  const cleanCompletedAt = Date.parse(cleanMachine.completed_at);
  const requiredMacosMatrix = new Set(["macos-14-latest", "macos-15-latest", "macos-current"]);
  const requiredHelperProbes = new Set([
    "voice_capture",
    "vision_ocr",
    "pdf_build_render",
    "local_inference",
    "pdf_extraction",
  ]);
  const matrixRows = Array.isArray(cleanMachine.os_matrix) ? cleanMachine.os_matrix : [];
  const matrixValid =
    matrixRows.length === requiredMacosMatrix.size &&
    matrixRows.every((row) => {
      if (!requiredMacosMatrix.delete(row.target) || row.passed !== true || row.synthetic !== false) {
        return false;
      }
      const probes = new Set(
        Array.isArray(row.helper_probes)
          ? row.helper_probes.filter((probe) => probe.passed === true).map((probe) => probe.name)
          : [],
      );
      return [...requiredHelperProbes].every((name) => probes.has(name));
    }) &&
    requiredMacosMatrix.size === 0;
  validateInstalledIntegrityEvidence(cleanMachine, {
    expectedBuildNumber,
    expectedBundleIdentifier,
    manifestPayloadSha256,
    installedAppSubtree,
    expectedManifestSha256,
    expectedAppTreeSha256,
    expectedTeamId,
    expectedDesignatedRequirementSha256,
  });
  if (
    cleanMachine.status !== "passed" ||
    cleanMachine.synthetic !== false ||
    cleanMachine.build_identifier !== buildId ||
    cleanMachine.source_revision !== sourceRevision ||
    cleanMachine.artifact_identifier !== artifactId ||
    cleanMachine.artifact_digest !== artifactDigest ||
    cleanMachine.artifact_file_sha256 !== dmgSha256 ||
    !cleanMachine.machine_identifier ||
    !cleanMachine.endpoint ||
    !cleanMachine.installed_application ||
    basename(cleanMachine.launched_executable ?? "") !== mainBinaryName ||
    cleanMachine.launch_exit_code !== 0 ||
    cleanMachine.installed_from_dmg !== true ||
    cleanMachine.codesign_verified !== true ||
    cleanMachine.stapled_ticket_verified !== true ||
    cleanMachine.repository_present !== false ||
    cleanMachine.repository_scripts_on_path !== false ||
    !matrixValid ||
    !Number.isFinite(cleanCompletedAt) ||
    cleanCompletedAt > Date.now() + 60_000 ||
    cleanCompletedAt < Date.now() - 24 * 60 * 60 * 1000
  ) {
    throw new Error(
      "release_lab_integrity_evidence_incomplete: clean-machine runner did not attest a real launch of this exact artifact",
    );
  }
}

function prepareCleanMachineCandidate(context) {
  const installedAppSubtree = context.manifest.subtrees?.find(
    (subtree) => subtree.path_prefix === basename(context.stagedApp),
  );
  if (!installedAppSubtree) {
    throw new Error(
      "release_manifest_app_subtree_missing: signed manifest has no independently verifiable application subtree",
    );
  }
  const mainIdentity = context.candidateIntegrity.codeObjects?.find(
    (entry) => entry.path === ".",
  );
  if (!mainIdentity?.designatedRequirementSha256) {
    throw new Error(
      "release_candidate_identity_incomplete: the verified candidate has no main designated requirement",
    );
  }
  const expectedManifestSha256 = context.sha256(readFileSync(context.manifestPath));
  const expectedAppTreeSha256 = context.candidateIntegrity.application.treeDigest.replace(
    /^sha256:/u,
    "",
  );
  const cleanMachineSandbox = join(context.workDir, "clean-machine-under-test");
  mkdirSync(cleanMachineSandbox, { recursive: false, mode: 0o700 });
  const cleanMachineDmg = join(cleanMachineSandbox, basename(context.stagedDmg));
  copyFileSync(context.stagedDmg, cleanMachineDmg);
  chmodSync(cleanMachineDmg, 0o400);
  if (context.sha256(readFileSync(cleanMachineDmg)) !== context.dmgSha256) {
    throw new Error("Clean-machine test copy does not match the immutable candidate DMG.");
  }
  return {
    cleanMachineDmg,
    cleanMachineSandbox,
    expectedAppTreeSha256,
    expectedManifestSha256,
    installedAppSubtree,
    mainIdentity,
  };
}

function runCleanMachineLaunch(context, prepared) {
  const cleanMachineRawPath = join(context.rawEvidenceDir, "clean-machine-launch.json");
  context.runStep("clean_machine_launch", context.cleanMachineRunner, [
    "--artifact", prepared.cleanMachineDmg,
    "--build-id", context.buildId,
    "--source-revision", context.sourceRevision,
    "--artifact-id", context.artifactId,
    "--artifact-digest", context.artifactDigest,
    "--artifact-file-sha256", context.dmgSha256,
    "--manifest-payload-sha256", context.manifest.payload_sha256,
    "--installed-app-prefix", prepared.installedAppSubtree.path_prefix,
    "--installed-app-subtree-digest", prepared.installedAppSubtree.artifact_digest,
    "--installed-app-entry-count", String(prepared.installedAppSubtree.entry_count),
    "--manifest-sha256", prepared.expectedManifestSha256,
    "--app-tree-sha256", prepared.expectedAppTreeSha256,
    "--bundle-identifier", context.expectedBundleIdentifier,
    "--build-number", String(context.expectedBuildNumber),
    "--team-id", context.candidateIntegrity.application.teamId,
    "--designated-requirement-sha256", prepared.mainIdentity.designatedRequirementSha256,
    "--output", cleanMachineRawPath,
  ], {
    cwd: prepared.cleanMachineSandbox,
    env: context.externalHarnessEnvironment(),
    echo: false,
    suppressFailureOutput: true,
  });
  const cleanMachine = context.readJson(cleanMachineRawPath, "Clean-machine launch report");
  validateCleanMachineReport({
    cleanMachine,
    buildId: context.buildId,
    sourceRevision: context.sourceRevision,
    artifactId: context.artifactId,
    artifactDigest: context.artifactDigest,
    dmgSha256: context.dmgSha256,
    mainBinaryName: context.mainBinaryName,
    expectedBuildNumber: context.expectedBuildNumber,
    expectedBundleIdentifier: context.expectedBundleIdentifier,
    manifestPayloadSha256: context.manifest.payload_sha256,
    installedAppSubtree: prepared.installedAppSubtree,
    expectedManifestSha256: prepared.expectedManifestSha256,
    expectedAppTreeSha256: prepared.expectedAppTreeSha256,
    expectedTeamId: context.candidateIntegrity.application.teamId,
    expectedDesignatedRequirementSha256: prepared.mainIdentity.designatedRequirementSha256,
  });
  if (context.sha256(readFileSync(prepared.cleanMachineDmg)) !== context.dmgSha256) {
    throw new Error("Clean-machine harness changed the exact DMG copy under test.");
  }
  return cleanMachine;
}

function runP0Acceptance(context, prepared) {
  const outputDir = join(context.rawEvidenceDir, "p0-acceptance");
  mkdirSync(outputDir, { recursive: false, mode: 0o700 });
  context.runStep("p0_acceptance_external", context.p0AcceptanceRunner, [
    "--artifact", prepared.cleanMachineDmg,
    "--build-id", context.buildId,
    "--source-revision", context.sourceRevision,
    "--artifact-id", context.artifactId,
    "--artifact-digest", context.artifactDigest,
    "--output-dir", outputDir,
  ], {
    cwd: prepared.cleanMachineSandbox,
    env: context.externalHarnessEnvironment(),
    echo: false,
    suppressFailureOutput: true,
  });
  context.runStep("p0_acceptance_validation", context.node, [
    "scripts/p0-release-acceptance.mjs",
    `--evidence-dir=${outputDir}`,
    `--build-id=${context.buildId}`,
    `--source-revision=${context.sourceRevision}`,
    `--artifact-digest=${context.artifactDigest}`,
  ]);
  return {
    goldenTaskMatrix: context.readJson(
      join(outputDir, "golden-task-runs.json"), "Golden-task matrix"),
    recoveryMatrix: context.readJson(
      join(outputDir, "recovery-matrix.json"), "Recovery matrix"),
    heroWorkflowEvidence: context.readJson(
      join(outputDir, "hero-workflow-runs.json"), "Hero workflow evidence"),
    privacyDeclarations: context.readJson(
      join(outputDir, "privacy-declarations.json"), "Privacy declarations"),
  };
}

function verifyImmutableCandidate(context) {
  context.makeTreeImmutable(context.candidateDir);
  context.runStep("manifest_verification_final", context.node, [
    "scripts/release-manifest.mjs", "verify",
    "--tree", context.candidateDir,
    "--manifest", context.manifestPath,
    "--build-id", context.buildId,
    "--source-revision", context.sourceRevision,
    "--artifact-id", context.artifactId,
    "--public-key", context.publicKeyPath,
    "--provenance", context.releaseProvenancePath,
  ]);
  if (context.sha256(readFileSync(context.stagedDmg)) !== context.dmgSha256) {
    throw new Error("Canonical DMG changed after exact-tree manifest generation.");
  }
  context.runStep("verify_immutable_candidate_codesign", context.codesign, [
    "--verify", "--deep", "--strict", "--verbose=4", context.stagedApp,
  ]);
  context.runStep("verify_immutable_dmg_codesign", context.codesign, [
    "--verify", "--strict", "--verbose=4", context.stagedDmg,
  ]);
  context.runStep("verify_immutable_app_ticket", "/usr/bin/xcrun", [
    "stapler", "validate", context.stagedApp,
  ]);
  context.runStep("verify_immutable_dmg_ticket", "/usr/bin/xcrun", [
    "stapler", "validate", context.stagedDmg,
  ]);
}

export function runCleanMachineQualification(context) {
  const prepared = prepareCleanMachineCandidate(context);
  const cleanMachine = runCleanMachineLaunch(context, prepared);
  const acceptance = runP0Acceptance(context, prepared);
  verifyImmutableCandidate(context);

  return {
    cleanMachine,
    cleanMachineDmg: prepared.cleanMachineDmg,
    ...acceptance,
  };
}
