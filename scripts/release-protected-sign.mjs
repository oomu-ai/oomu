#!/usr/bin/env node

import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { mkdtempSync, tmpdir } from "node:os";
import { basename, join, relative, resolve } from "node:path";
import process from "node:process";
import { verifyAndExpandUnsignedHandoff } from "./release-handoff.mjs";
import {
  artifactDigestForEntries,
  collectTreeEntries,
  generateReleaseManifest,
  verifyReleaseManifest,
} from "./release-manifest.mjs";
import {
  collectReleaseToolchain,
  runApproved,
  sha256Bytes,
} from "./release-provenance.mjs";
import {
  finalizeProtectedApplicationQualification,
  prepareProtectedApplicationQualification,
  qualifySignedApplication,
  qualifySignedCandidate,
} from "./release-protected-qualification.mjs";
import {
  loadReleaseVersionRecord,
  releaseArtifactIdentifier,
  releaseDmgName,
} from "./release-version.mjs";
import {
  assertSignedArtifactUnchanged,
  orderedNestedCodeTargets,
  signedArtifactIdentity,
} from "./release-signing-order.mjs";

const root = resolve(import.meta.dirname, "..");
const tauriConfig = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const releaseVersion = loadReleaseVersionRecord(root);

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required in the protected release phase.`);
  return value;
}

function makeTreeImmutable(rootPath) {
  const protect = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        protect(child);
        chmodSync(child, statSync(child).mode & 0o555);
      } else if (entry.isFile()) {
        const mode = statSync(child).mode;
        chmodSync(child, mode & 0o111 ? 0o555 : 0o444);
      }
    }
  };
  protect(rootPath);
  chmodSync(rootPath, 0o555);
}

function codesignArguments(identity, path, entitlements = null, hardenedRuntime = true) {
  const args = ["--force", "--timestamp"];
  if (hardenedRuntime) args.push("--options", "runtime");
  args.push("--sign", identity);
  if (entitlements) args.push("--entitlements", entitlements);
  args.push(path);
  return args;
}

function clearGeneratedBundleExtendedAttributes(toolchain, appPath) {
  runApproved(toolchain, "xattr", ["-cr", appPath], {
    label: "clear generated bundle extended attributes",
  });
}

function signNestedCode(toolchain, appPath, identity) {
  const targets = orderedNestedCodeTargets(
    appPath,
    toolchain.tools.file.executable,
    (path) => runApproved(toolchain, "file", ["-b", path], {
      label: `classify ${relative(appPath, path)}`,
    }).stdout,
  );
  for (const target of targets) {
    runApproved(toolchain, "codesign", codesignArguments(identity, target, null), {
      label: `sign nested code ${relative(appPath, target)}`,
    });
  }
  return targets;
}

function assertOnlySigningMutations(
  unsignedEntries,
  signedEntries,
  appPath,
  signedTargets,
  authorizedResources,
) {
  const before = new Map(unsignedEntries.map((entry) => [entry.path, entry]));
  const after = new Map(signedEntries.map((entry) => [entry.path, entry]));
  const directlySigned = new Set(
    signedTargets
      .filter((path) => statSync(path).isFile())
      .map((path) => relative(appPath, path)),
  );
  const attestationResources = new Set(
    authorizedResources.map((path) => relative(appPath, path)),
  );
  const isSignatureMetadata = (path) =>
    path === "Contents/CodeResources" ||
    path.includes("/_CodeSignature/") ||
    path.endsWith("/_CodeSignature/CodeResources");
  const unexpected = [];
  for (const [path, entry] of before) {
    const signed = after.get(path);
    if (!signed) unexpected.push(`removed:${path}`);
    else if (
      JSON.stringify(entry) !== JSON.stringify(signed) &&
      !directlySigned.has(path) &&
      !attestationResources.has(path) &&
      !isSignatureMetadata(path)
    ) unexpected.push(`changed:${path}`);
  }
  for (const path of after.keys()) {
    if (
      !before.has(path) &&
      !attestationResources.has(path) &&
      !isSignatureMetadata(path)
    ) unexpected.push(`added:${path}`);
  }
  if (unexpected.length > 0) {
    throw new Error(
      `Protected signing changed unsigned content outside the reviewed effects: ${unexpected.join(", ")}`,
    );
  }
  return [...new Set([
    ...directlySigned,
    ...attestationResources,
    ...[...after.keys()].filter(isSignatureMetadata),
  ])].sort();
}

function notarize(toolchain, path, credentials) {
  const args = ["submit", path, "--wait", "--output-format", "json"];
  if (credentials.mode === "apple-id") {
    args.push(
      "--apple-id", credentials.appleId,
      "--password", credentials.password,
      "--team-id", credentials.teamId,
    );
  } else {
    args.push(
      "--issuer", credentials.issuer,
      "--key-id", credentials.keyId,
      "--key", credentials.keyPath,
    );
  }
  const result = runApproved(toolchain, "notarytool", args, {
    label: `notarize ${basename(path)}`,
  });
  const receipt = JSON.parse(result.stdout);
  if (receipt.status !== "Accepted" || !receipt.id) {
    throw new Error(`Apple did not accept ${basename(path)} for notarization.`);
  }
  return { id: receipt.id, status: receipt.status };
}

function signingIdentityDetails(toolchain, appPath, expectedTeamId) {
  const result = runApproved(toolchain, "codesign", ["-d", "--verbose=4", appPath], {
    label: "inspect signed application identity",
  });
  const details = `${result.stdout}\n${result.stderr}`;
  const teamId = details.match(/^TeamIdentifier=(.+)$/mu)?.[1]?.trim();
  const identifier = details.match(/^Identifier=(.+)$/mu)?.[1]?.trim();
  const authority = details.match(/^Authority=(.+)$/mu)?.[1]?.trim();
  if (
    teamId !== expectedTeamId ||
    identifier !== tauriConfig.identifier ||
    !authority?.startsWith("Developer ID Application:") ||
    !/flags=.*\(runtime\)/u.test(details)
  ) throw new Error("Signed application identity does not match the reviewed release identity.");
  return { teamId, identifier, authority, hardenedRuntime: true };
}

function loadVerifiedHandoff() {
  if (process.env.GITHUB_ACTIONS !== "true") {
    throw new Error("Protected signing must run in the reviewed protected CI job.");
  }
  const toolchain = collectReleaseToolchain({ ciPhase: true, protectedPhase: true });
  const buildIdentifier = required("OOMU_BUILD_ID");
  const sourceRevision = required("OOMU_SOURCE_REVISION");
  const outputDirectory = resolve(required("OOMU_PROTECTED_RELEASE_DIR"));
  if (existsSync(outputDirectory)) throw new Error("Protected release output must be new.");
  const verified = verifyAndExpandUnsignedHandoff({
    handoffPath: resolve(required("OOMU_UNSIGNED_HANDOFF_PATH")),
    archivePath: resolve(required("OOMU_UNSIGNED_ARCHIVE_PATH")),
    expectedHandoffSha256: required("OOMU_UNSIGNED_HANDOFF_SHA256"),
    expectedBuildIdentifier: buildIdentifier,
    expectedSourceRevision: sourceRevision,
    toolchain,
  });
  return {
    ...verified,
    toolchain,
    buildIdentifier,
    mainBinaryName: tauriConfig.mainBinaryName,
    packageVersion: releaseVersion.productVersion,
    releaseVersion,
    sourceRevision,
    outputDirectory,
  };
}

function releaseCredentials(teamId) {
  if (process.env.APPLE_ID && process.env.APPLE_PASSWORD) {
    return {
      mode: "apple-id",
      appleId: process.env.APPLE_ID,
      password: process.env.APPLE_PASSWORD,
      teamId,
    };
  }
  return {
    mode: "api-key",
    issuer: required("APPLE_API_ISSUER"),
    keyId: required("APPLE_API_KEY"),
    keyPath: resolve(required("APPLE_API_KEY_PATH")),
  };
}

function verifySigningAuthority(toolchain) {
  const names = [
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_KEY_PATH",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_TEAM_ID",
  ];
  const environment = Object.fromEntries(
    names
      .filter((name) => process.env[name] !== undefined)
      .map((name) => [name, process.env[name]]),
  );
  runApproved(toolchain, "node", ["scripts/preflight_signing.js"], {
    label: "verify protected signing authority",
    environment,
  });
}

function signAndNotarizeApplication(context, signingIdentity, teamId, credentials) {
  const { appPath, toolchain } = context;
  clearGeneratedBundleExtendedAttributes(toolchain, appPath);
  const unsignedEntries = collectTreeEntries(appPath);
  const preparedQualification = prepareProtectedApplicationQualification(context);
  const nestedTargets = signNestedCode(toolchain, appPath, signingIdentity);
  const protectedResources = finalizeProtectedApplicationQualification(
    context,
    preparedQualification,
    teamId,
  );
  runApproved(toolchain, "codesign", codesignArguments(
    signingIdentity,
    appPath,
    join(root, "src-tauri", "entitlements.plist"),
  ), { label: "sign application" });
  runApproved(toolchain, "codesign", [
    "--verify", "--deep", "--strict", "--verbose=4", appPath,
  ], { label: "verify signed application" });
  const signedApplicationIdentity = signedArtifactIdentity(appPath);
  const signing = signingIdentityDetails(toolchain, appPath, teamId);
  const workDirectory = mkdtempSync(join(tmpdir(), "oomu-protected-signing-"));
  const appArchive = join(workDirectory, `${tauriConfig.productName}.zip`);
  assertSignedArtifactUnchanged(
    appPath, signedApplicationIdentity, "before_protected_notarization_archive",
  );
  runApproved(toolchain, "ditto", ["-c", "-k", "--keepParent", appPath, appArchive], {
    label: "archive application for notarization",
  });
  const notarization = notarize(toolchain, appArchive, credentials);
  assertSignedArtifactUnchanged(
    appPath, signedApplicationIdentity, "after_protected_notarization",
  );
  runApproved(toolchain, "stapler", ["staple", appPath], { label: "staple application" });
  runApproved(toolchain, "stapler", ["validate", appPath], { label: "validate app ticket" });
  runApproved(toolchain, "codesign", [
    "--verify", "--deep", "--strict", "--verbose=4", appPath,
  ], { label: "verify stapled application" });
  const finalApplicationIdentity = signedArtifactIdentity(appPath);
  const mainBinary = join(appPath, "Contents", "MacOS", tauriConfig.mainBinaryName);
  const authorizedMutations = assertOnlySigningMutations(
    unsignedEntries,
    collectTreeEntries(appPath),
    appPath,
    [...nestedTargets, mainBinary],
    protectedResources.authorizedResourcePaths,
  );
  return {
    authorizedMutations,
    notarization,
    preparedQualification,
    protectedResources,
    signing,
    finalApplicationIdentity,
    workDirectory,
  };
}

function createAndNotarizeDmg(
  context,
  signingIdentity,
  credentials,
  workDirectory,
  expectedApplicationIdentity,
) {
  const { appPath, toolchain } = context;
  assertSignedArtifactUnchanged(
    appPath, expectedApplicationIdentity, "before_protected_dmg_creation",
  );
  const dmgRoot = join(workDirectory, "dmg-root");
  mkdirSync(dmgRoot, { recursive: false, mode: 0o700 });
  runApproved(toolchain, "ditto", [appPath, join(dmgRoot, basename(appPath))], {
    label: "stage signed application for DMG",
  });
  const dmgPath = join(
    workDirectory,
    releaseDmgName(context.releaseVersion, tauriConfig.productName),
  );
  runApproved(toolchain, "hdiutil", [
    "create", "-volname", tauriConfig.productName, "-srcfolder", dmgRoot,
    "-ov", "-format", "UDZO", dmgPath,
  ], { label: "create release DMG" });
  runApproved(toolchain, "codesign", codesignArguments(signingIdentity, dmgPath, null, false), {
    label: "sign release DMG",
  });
  const notarization = notarize(toolchain, dmgPath, credentials);
  runApproved(toolchain, "stapler", ["staple", dmgPath], { label: "staple DMG" });
  runApproved(toolchain, "stapler", ["validate", dmgPath], { label: "validate DMG ticket" });
  runApproved(toolchain, "spctl", ["--assess", "--type", "execute", "--verbose=4", appPath], {
    label: "assess signed application",
  });
  const architecture = runApproved(toolchain, "lipo", [
    "-archs", join(appPath, "Contents", "MacOS", tauriConfig.mainBinaryName),
  ], { label: "verify application architecture" }).stdout.trim();
  if (architecture !== "arm64") throw new Error("Signed application architecture is not arm64.");
  assertSignedArtifactUnchanged(
    appPath, expectedApplicationIdentity, "after_protected_dmg_creation",
  );
  return { dmgPath, notarization };
}

function stageCandidate(context, application, dmg) {
  const { appPath, handoff, outputDirectory, toolchain } = context;
  const candidateDirectory = join(outputDirectory, "candidate");
  mkdirSync(candidateDirectory, { recursive: true, mode: 0o700 });
  const stagedApp = join(candidateDirectory, basename(appPath));
  const stagedDmg = join(candidateDirectory, basename(dmg.dmgPath));
  assertSignedArtifactUnchanged(
    appPath, application.finalApplicationIdentity, "before_protected_candidate_staging",
  );
  runApproved(toolchain, "ditto", [appPath, stagedApp], { label: "stage final application" });
  assertSignedArtifactUnchanged(
    appPath, application.finalApplicationIdentity, "after_protected_candidate_staging",
  );
  copyFileSync(dmg.dmgPath, stagedDmg);
  chmodSync(stagedDmg, 0o400);
  const signedOutputDigest = artifactDigestForEntries(collectTreeEntries(candidateDirectory));
  const provenance = {
    schemaVersion: 1,
    kind: "oomu.release-provenance",
    releaseVersion: context.releaseVersion,
    workflowSourceCommit: context.sourceRevision,
    releasePolicyId: toolchain.policy.policyId,
    releasePolicyDigest: toolchain.policyDigest,
    runnerIdentity: toolchain.runner,
    actionCommitShas: toolchain.policy.actions,
    executableEvidence: toolchain.tools,
    executableVersions: toolchain.versions,
    rustToolchain: toolchain.policy.rust,
    xcodeSdk: toolchain.runner.xcode,
    unsignedHandoffIdentity: handoff.artifactIdentity,
    unsignedTreeDigest: handoff.unsignedTreeDigest,
    unsignedPayloadDigest: handoff.unsignedPayloadDigest,
    unsignedArchiveDigest: handoff.archiveSha256,
    unsignedBuilderToolchain: handoff.toolchain,
    unsignedLockDigests: handoff.lockDigests,
    unsignedGatePolicyDigests: handoff.gatePolicyDigests,
    protectedSignerToolchain: { tools: toolchain.tools, versions: toolchain.versions },
    signedOutputDigest,
    signedDmgSha256: sha256Bytes(readFileSync(stagedDmg)),
    buildSignPhaseIsolated: true,
    authorizedSigningMutations: application.authorizedMutations,
    signing: application.signing,
    notarization: { app: application.notarization, dmg: dmg.notarization },
    protectedApplicationQualification: {
      runtime: application.protectedResources.evidence,
      signedApplication: application.signedApplicationQualification,
    },
    generatedAt: new Date().toISOString(),
  };
  return {
    candidateDirectory,
    provenance,
    signedOutputDigest,
    stagedApp,
    stagedDmg,
  };
}

function writeBoundEvidence(context, staged) {
  const privateKeyPath = resolve(required("OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH"));
  const publicKeyPath = resolve(required("OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH"));
  const provenancePath = join(context.outputDirectory, "release-provenance.json");
  writeFileSync(provenancePath, `${JSON.stringify(staged.provenance, null, 2)}\n`, {
    mode: 0o400,
    flag: "wx",
  });
  const manifestPath = join(context.outputDirectory, "MANIFEST.json");
  const artifactIdentifier = releaseArtifactIdentifier(
    context.releaseVersion,
    context.buildIdentifier,
  );
  generateReleaseManifest({
    treeRoot: staged.candidateDirectory,
    manifestPath,
    buildIdentifier: context.buildIdentifier,
    sourceRevision: context.sourceRevision,
    artifactIdentifier,
    privateKeyPath,
    releaseProvenance: staged.provenance,
  });
  verifyReleaseManifest({
    treeRoot: staged.candidateDirectory,
    manifestPath,
    expectedBuildIdentifier: context.buildIdentifier,
    expectedSourceRevision: context.sourceRevision,
    expectedArtifactIdentifier: artifactIdentifier,
    publicKeyPath,
    expectedReleaseProvenance: staged.provenance,
  });
  return { manifestPath, provenancePath };
}

function verifyImmutableCandidate(context, staged) {
  const entitlementReportPath = join(
    context.outputDirectory,
    "release-candidate-entitlements.json",
  );
  runApproved(context.toolchain, "node", [
    "scripts/check-entitlements.mjs",
    "--signed-app", staged.stagedApp,
    "--output", entitlementReportPath,
  ], { label: "verify final semantic entitlements" });
  const evidencePath = join(
    context.outputDirectory,
    "release-candidate-integrity.json",
  );
  runApproved(context.toolchain, "node", [
    "scripts/release-candidate-integrity.mjs",
    "--app", staged.stagedApp,
    "--container", staged.stagedDmg,
    "--entitlements", entitlementReportPath,
    "--output", evidencePath,
    "--team-id", required("APPLE_TEAM_ID"),
    "--bundle-id", tauriConfig.identifier,
    "--build-number", String(context.releaseVersion.buildNumber),
    "--codesign", context.toolchain.tools.codesign.executable,
    "--file", context.toolchain.tools.file.executable,
    "--plutil", context.toolchain.tools.plutil.executable,
    "--spctl", context.toolchain.tools.spctl.executable,
    "--xcrun", context.toolchain.tools.xcrun.executable,
  ], { label: "verify immutable release candidate" });
  const report = JSON.parse(readFileSync(evidencePath, "utf8"));
  if (
    report.kind !== "oomu.release-candidate-integrity" ||
    report.status !== "passed" ||
    report.synthetic !== false
  ) throw new Error("Immutable candidate verification did not produce real passing evidence.");
  return { evidencePath, report };
}

function main() {
  const context = loadVerifiedHandoff();
  if (process.argv.includes("--verify-only")) {
    process.stdout.write(`Verified unsigned handoff ${context.handoff.artifactIdentity}.\n`);
    return;
  }
  const signingIdentity = required("APPLE_SIGNING_IDENTITY");
  const teamId = required("APPLE_TEAM_ID");
  const credentials = releaseCredentials(teamId);
  verifySigningAuthority(context.toolchain);
  const application = signAndNotarizeApplication(
    context, signingIdentity, teamId, credentials,
  );
  application.signedApplicationQualification = qualifySignedApplication(
    context,
    application.preparedQualification,
    application.signing,
    teamId,
  );
  const dmg = createAndNotarizeDmg(
    context,
    signingIdentity,
    credentials,
    application.workDirectory,
    application.finalApplicationIdentity,
  );
  const staged = stageCandidate(context, application, dmg);
  const integrity = verifyImmutableCandidate(context, staged);
  staged.provenance.releaseCandidateIntegrity = {
    kind: integrity.report.kind,
    schemaVersion: integrity.report.schemaVersion,
    reportSha256: sha256Bytes(readFileSync(integrity.evidencePath)),
    applicationTreeDigest: integrity.report.application.treeDigest,
    codeObjectCount: integrity.report.application.codeObjectCount,
    bundleIdentifier: integrity.report.application.bundleIdentifier,
    teamId: integrity.report.application.teamId,
    authority: integrity.report.application.authority,
    buildNumber: integrity.report.application.buildNumber,
    codeDirectoryHash: integrity.report.application.codeDirectoryHash,
    designatedRequirementSha256:
      integrity.report.application.designatedRequirementSha256,
    hardenedRuntime: integrity.report.application.hardenedRuntime,
    entitlementDigest: integrity.report.entitlements.canonicalSha256,
    gatekeeperAccepted:
      integrity.report.application.gatekeeperAccepted
      && integrity.report.container.gatekeeperAccepted,
    notarizationAccepted:
      integrity.report.application.notarizationTicketValidated &&
      integrity.report.container.notarizationTicketValidated,
  };
  makeTreeImmutable(staged.candidateDirectory);
  const evidence = writeBoundEvidence(context, staged);
  const protectedCandidateQualification = qualifySignedCandidate(
    context,
    staged,
    evidence.manifestPath,
    integrity.report,
  );
  const qualificationPath = join(
    context.outputDirectory,
    "protected-candidate-qualification.json",
  );
  writeFileSync(
    qualificationPath,
    `${JSON.stringify(protectedCandidateQualification, null, 2)}\n`,
    { mode: 0o400, flag: "wx" },
  );
  process.stdout.write(`OOMU_PROTECTED_RELEASE_DIGEST=${staged.signedOutputDigest}\n`);
  process.stdout.write(`OOMU_RELEASE_CANDIDATE_MANIFEST=${evidence.manifestPath}\n`);
  process.stdout.write(`OOMU_RELEASE_CANDIDATE_EVIDENCE=${integrity.evidencePath}\n`);
  process.stdout.write(`OOMU_RELEASE_QUALIFICATION_EVIDENCE=${qualificationPath}\n`);
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU PROTECTED RELEASE FAILED: ${error.message}`);
    process.exit(1);
  }
}
