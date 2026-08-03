#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  readFileSync,
  realpathSync,
} from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import {
  artifactDigestForEntries,
  atomicWriteFile,
  collectTreeEntries,
  verifyReleaseManifestSubtree,
} from "./release-manifest.mjs";
import {
  codeSigningOrder,
  signedArtifactIdentity,
} from "./release-signing-order.mjs";

export const CANDIDATE_EVIDENCE_KIND = "oomu.release-candidate-integrity";
export const CANDIDATE_EVIDENCE_SCHEMA_VERSION = 2;

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stableValue(value) {
  if (Array.isArray(value)) return value.map(stableValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, stableValue(child)]),
    );
  }
  return value;
}

function run(executable, args, options = {}) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    input: options.input,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    throw new Error(
      `${basename(executable)} ${args[0] ?? ""} failed: ${(result.stderr || result.stdout || "no output").trim()}`,
    );
  }
  return result;
}

function requireMacTool(path, label) {
  if (process.platform !== "darwin") {
    throw new Error("not executed - environment not ready: macOS is required");
  }
  const candidate = resolve(path);
  if (!existsSync(candidate)) {
    throw new Error(`not executed - environment not ready: ${label} is unavailable`);
  }
  return realpathSync(candidate);
}

export function parseCodeSignatureDetails(text) {
  const value = (prefix) => text
    .split(/\r?\n/u)
    .find((line) => line.startsWith(prefix))
    ?.slice(prefix.length)
    .trim() || null;
  return {
    bundleIdentifier: value("Identifier="),
    teamId: value("TeamIdentifier="),
    authority: value("Authority="),
    codeDirectoryHash: value("CDHash="),
    designatedRequirementSha256: value("designated => ")
      ? sha256(value("designated => "))
      : null,
    hardenedRuntime: text
      .split(/\r?\n/u)
      .some((line) => line.startsWith("CodeDirectory ") && /flags=.*\(runtime\)/u.test(line)),
  };
}

export function validateEntitlementReport(report) {
  const digest = report?.application?.extracted?.canonical_sha256;
  if (
    report?.kind !== "oomu.final-signed-entitlement-snapshot" ||
    report?.status !== "passed" ||
    report?.synthetic !== false ||
    !/^[0-9a-f]{64}$/u.test(digest ?? "") ||
    digest !== report?.application?.reviewed_policy?.canonical_sha256
  ) {
    throw new Error("Signed application entitlements were not semantically verified.");
  }
  return digest;
}

export function strictCodesignVerification(codesign, target, options = {}) {
  const before = signedArtifactIdentity(target);
  const args = ["--verify"];
  if (options.deep) args.push("--deep");
  args.push("--strict", "--verbose=4", target);
  const result = run(codesign, args, { allowFailure: true });
  const after = signedArtifactIdentity(target);
  const exitStatus = Number.isInteger(result.status) ? result.status : null;
  let failureCode = null;
  if (before !== after) failureCode = "release_signature_artifact_changed_during_verification";
  else if (exitStatus === null) failureCode = "release_signature_verifier_terminated";
  else if (exitStatus !== 0) failureCode = "release_signature_verification_failed";
  return {
    strictSignatureValid: failureCode === null,
    verifierExitStatus: exitStatus,
    failureCode,
    artifactIdentity: after,
  };
}

function requireStrictVerification(verification, target, appPath) {
  if (!verification.strictSignatureValid) {
    throw new Error(
      `${verification.failureCode}:${relative(appPath, target) || "."}:exit=${verification.verifierExitStatus ?? "unavailable"}`,
    );
  }
  return verification;
}

function parsedEntitlements(codesign, plutil, path) {
  const extracted = run(codesign, ["-d", "--entitlements", ":-", path], {
    allowFailure: true,
  });
  const plist = extracted.stdout.trim();
  if (!plist) return { keyCount: 0, canonicalSha256: sha256("{}") };
  const converted = run(
    plutil,
    ["-convert", "json", "-o", "-", "--", "-"],
    { input: plist },
  );
  const value = JSON.parse(converted.stdout);
  if (!value || Array.isArray(value) || typeof value !== "object") {
    throw new Error(`Signed entitlements are malformed for ${basename(path)}.`);
  }
  const canonical = JSON.stringify(stableValue(value));
  return { keyCount: Object.keys(value).length, canonicalSha256: sha256(canonical) };
}

export function validateCodeEntitlementPolicy({
  mainApplication,
  bundleIdentifier,
  expectedBundleIdentifier,
  entitlements,
  reviewedEntitlementDigest,
}) {
  const matchesReviewedApplication =
    bundleIdentifier === expectedBundleIdentifier
    && entitlements.canonicalSha256 === reviewedEntitlementDigest;
  if (mainApplication && !matchesReviewedApplication) {
    throw new Error("The signed application entitlements differ from the reviewed policy.");
  }
  if (!mainApplication && !matchesReviewedApplication && entitlements.keyCount !== 0) {
    throw new Error("Nested code has unreviewed entitlements.");
  }
}

export function verifiedApplicationBundleIdentifier(appIdentity, expectedBundleIdentifier) {
  if (appIdentity?.path !== "." || appIdentity.identifier !== expectedBundleIdentifier) {
    throw new Error("The verified application bundle identifier is missing or unexpected.");
  }
  return appIdentity.identifier;
}

function inspectCodeObject({
  appPath,
  target,
  codesign,
  plutil,
  expectedTeamId,
  expectedBundleIdentifier,
  reviewedEntitlementDigest,
}) {
  const verification = requireStrictVerification(
    strictCodesignVerification(codesign, target), target, appPath,
  );
  const detail = run(codesign, ["-d", "--verbose=4", "-r-", target]);
  const parsed = parseCodeSignatureDetails(`${detail.stdout}\n${detail.stderr}`);
  const mainApplication = target === appPath;
  if (
    parsed.teamId !== expectedTeamId ||
    !parsed.authority?.startsWith("Developer ID Application:") ||
    !parsed.codeDirectoryHash ||
    !parsed.designatedRequirementSha256 ||
    !parsed.hardenedRuntime ||
    (mainApplication && parsed.bundleIdentifier !== expectedBundleIdentifier)
  ) {
    throw new Error(`Code identity is incomplete or unexpected for ${relative(appPath, target) || "."}.`);
  }
  const entitlements = parsedEntitlements(codesign, plutil, target);
  try {
    validateCodeEntitlementPolicy({
      mainApplication,
      bundleIdentifier: parsed.bundleIdentifier,
      expectedBundleIdentifier,
      entitlements,
      reviewedEntitlementDigest,
    });
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`${detail} (${relative(appPath, target) || "."})`);
  }
  return {
    path: relative(appPath, target) || ".",
    objectType: lstatSync(target).isDirectory() ? "code_container" : "mach_o",
    identifier: parsed.bundleIdentifier,
    teamId: parsed.teamId,
    authority: parsed.authority,
    codeDirectoryHash: parsed.codeDirectoryHash,
    designatedRequirementSha256: parsed.designatedRequirementSha256,
    hardenedRuntime: parsed.hardenedRuntime,
    strictSignatureValid: verification.strictSignatureValid,
    verifierExitStatus: verification.verifierExitStatus,
    verificationFailureCode: verification.failureCode,
    artifactIdentity: verification.artifactIdentity,
    entitlements,
  };
}

function inspectContainerIdentity(codesign, container, expectedTeamId) {
  const detail = run(codesign, ["-d", "--verbose=4", "-r-", container]);
  const identity = parseCodeSignatureDetails(`${detail.stdout}\n${detail.stderr}`);
  if (
    identity.teamId !== expectedTeamId
    || !identity.authority?.startsWith("Developer ID Application:")
    || !identity.codeDirectoryHash
    || !identity.designatedRequirementSha256
  ) {
    throw new Error("Distribution container signing identity is incomplete or unexpected.");
  }
  return identity;
}

function stableVerifiedArtifactIdentities(
  application,
  container,
  applicationVerification,
  containerVerification,
) {
  const applicationTreeIdentity = signedArtifactIdentity(application);
  const containerArtifactIdentity = signedArtifactIdentity(container);
  if (applicationTreeIdentity !== applicationVerification.artifactIdentity) {
    throw new Error("release_signature_artifact_changed_after_verification:application");
  }
  if (containerArtifactIdentity !== containerVerification.artifactIdentity) {
    throw new Error("release_signature_artifact_changed_after_verification:container");
  }
  return { applicationTreeIdentity, containerArtifactIdentity };
}

export function verifyReleaseCandidate({
  appPath,
  containerPath,
  entitlementReportPath,
  outputPath,
  expectedTeamId,
  expectedBundleIdentifier,
  expectedBuildNumber,
  tools,
}) {
  const application = realpathSync(resolve(appPath));
  const container = realpathSync(resolve(containerPath));
  const codesign = requireMacTool(tools.codesign, "codesign");
  const file = requireMacTool(tools.file, "file");
  const plutil = requireMacTool(tools.plutil, "plutil");
  const spctl = requireMacTool(tools.spctl, "Gatekeeper");
  const xcrun = requireMacTool(tools.xcrun, "xcrun");
  const entitlementReport = JSON.parse(readFileSync(resolve(entitlementReportPath), "utf8"));
  const entitlementDigest = validateEntitlementReport(entitlementReport);

  const codeObjects = codeSigningOrder(application, file).map((target) => inspectCodeObject({
    appPath: application,
    target,
    codesign,
    plutil,
    expectedTeamId,
    expectedBundleIdentifier,
    reviewedEntitlementDigest: entitlementDigest,
  }));
  const applicationVerification = requireStrictVerification(
    strictCodesignVerification(codesign, application, { deep: true }),
    application,
    application,
  );
  const containerVerification = requireStrictVerification(
    strictCodesignVerification(codesign, container),
    container,
    application,
  );
  run(xcrun, ["stapler", "validate", application]);
  run(xcrun, ["stapler", "validate", container]);
  run(spctl, ["--assess", "--type", "execute", "--verbose=4", application]);
  run(spctl, [
    "--assess",
    "--type", "open",
    "--context", "context:primary-signature",
    "--verbose=4",
    container,
  ]);
  const buildNumber = run(plutil, [
    "-extract", "CFBundleVersion", "raw", "-o", "-",
    join(application, "Contents", "Info.plist"),
  ]).stdout.trim();
  if (buildNumber !== String(expectedBuildNumber)) {
    throw new Error("Installed application build number differs from the release request.");
  }
  const containerIdentity = inspectContainerIdentity(codesign, container, expectedTeamId);
  const { applicationTreeIdentity, containerArtifactIdentity } =
    stableVerifiedArtifactIdentities(
      application, container, applicationVerification, containerVerification,
    );

  const appIdentity = codeObjects.find((entry) => entry.path === ".");
  const evidence = {
    schemaVersion: CANDIDATE_EVIDENCE_SCHEMA_VERSION,
    kind: CANDIDATE_EVIDENCE_KIND,
    status: "passed",
    synthetic: false,
    completedAt: new Date().toISOString(),
    application: {
      fileName: basename(application),
      bundleIdentifier: verifiedApplicationBundleIdentifier(
        appIdentity, expectedBundleIdentifier),
      teamId: appIdentity.teamId,
      authority: appIdentity.authority,
      codeDirectoryHash: appIdentity.codeDirectoryHash,
      designatedRequirementSha256: appIdentity.designatedRequirementSha256,
      buildNumber,
      hardenedRuntime: true,
      strictSignatureValid: applicationVerification.strictSignatureValid,
      verifierExitStatus: applicationVerification.verifierExitStatus,
      verificationFailureCode: applicationVerification.failureCode,
      verifiedArtifactIdentity: applicationVerification.artifactIdentity,
      gatekeeperAccepted: true,
      notarizationTicketValidated: true,
      treeDigest: applicationTreeIdentity,
      codeObjectCount: codeObjects.length,
    },
    container: {
      fileName: basename(container),
      sha256: containerArtifactIdentity.replace(/^sha256:/u, ""),
      teamId: containerIdentity.teamId,
      authority: containerIdentity.authority,
      codeDirectoryHash: containerIdentity.codeDirectoryHash,
      designatedRequirementSha256: containerIdentity.designatedRequirementSha256,
      strictSignatureValid: containerVerification.strictSignatureValid,
      verifierExitStatus: containerVerification.verifierExitStatus,
      verificationFailureCode: containerVerification.failureCode,
      verifiedArtifactIdentity: containerVerification.artifactIdentity,
      gatekeeperAccepted: true,
      notarizationTicketValidated: true,
    },
    entitlements: {
      semanticComparison: "passed",
      canonicalSha256: entitlementDigest,
    },
    codeObjects,
  };
  if (outputPath) {
    atomicWriteFile(resolve(outputPath), `${JSON.stringify(evidence, null, 2)}\n`, 0o400);
  }
  return evidence;
}

export function inspectSignedCandidateAndEvidence({
  appPath,
  containerPath,
  entitlementReportPath,
  manifestPath,
  publicKeyPath,
  appPrefix,
  expectedTeamId,
  expectedBundleIdentifier,
  expectedBuildNumber,
  expectedBuildIdentifier,
  expectedSourceRevision,
  expectedArtifactIdentifier,
  outputPath,
  tools,
}) {
  const application = realpathSync(resolve(appPath));
  const beforeQualification = artifactDigestForEntries(collectTreeEntries(application));
  const candidate = verifyReleaseCandidate({
    appPath: application,
    containerPath,
    entitlementReportPath,
    outputPath,
    expectedTeamId,
    expectedBundleIdentifier,
    expectedBuildNumber,
    tools,
  });
  verifyReleaseManifestSubtree({
    treeRoot: application,
    manifestPath,
    publicKeyPath,
    pathPrefix: appPrefix,
    expectedBuildIdentifier,
    expectedSourceRevision,
    expectedArtifactIdentifier,
  });
  const afterQualification = artifactDigestForEntries(collectTreeEntries(application));
  if (beforeQualification !== afterQualification) {
    throw new Error("Candidate verification changed the signed application tree.");
  }
  const nestedExecutablesVerified = candidate.codeObjects.length > 1 &&
    candidate.codeObjects.every((entry) => entry.strictSignatureValid === true);
  if (!nestedExecutablesVerified) {
    throw new Error("Nested executable verification evidence is incomplete.");
  }
  return candidateEvidenceSummary({
    candidate,
    manifestPath,
    beforeQualification,
    afterQualification,
    nestedExecutablesVerified,
  });
}

export function candidateEvidenceSummary({
  candidate,
  manifestPath,
  beforeQualification,
  afterQualification,
  nestedExecutablesVerified,
}) {
  const mainIdentity = candidate.codeObjects.find((entry) => entry.path === ".");
  if (!mainIdentity?.designatedRequirementSha256) {
    throw new Error("The main application designated requirement is missing.");
  }
  return {
    bundleIdentifier: candidate.application.bundleIdentifier,
    channel: "production",
    buildNumber: Number(candidate.application.buildNumber),
    appTreeSha256: afterQualification.replace(/^sha256:/u, ""),
    manifestSha256: sha256(readFileSync(resolve(manifestPath))),
    codesignVerified: candidate.application.strictSignatureValid,
    gatekeeperAccepted: candidate.application.gatekeeperAccepted === true
      && candidate.container.gatekeeperAccepted === true,
    notarizationAccepted: candidate.application.notarizationTicketValidated &&
      candidate.container.notarizationTicketValidated,
    nestedExecutablesVerified,
    installedTreeMatches: true,
    teamId: candidate.application.teamId,
    designatedRequirementSha256: mainIdentity.designatedRequirementSha256,
    beforeQualificationSha256: beforeQualification.replace(/^sha256:/u, ""),
    afterQualificationSha256: afterQualification.replace(/^sha256:/u, ""),
  };
}

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("release-candidate-integrity requires named value arguments");
    }
    values[key.slice(2)] = value;
  }
  const required = (key) => {
    if (!values[key]) throw new Error(`--${key} is required`);
    return values[key];
  };
  return {
    appPath: required("app"),
    containerPath: required("container"),
    entitlementReportPath: required("entitlements"),
    outputPath: required("output"),
    expectedTeamId: required("team-id"),
    expectedBundleIdentifier: required("bundle-id"),
    expectedBuildNumber: required("build-number"),
    tools: {
      codesign: required("codesign"),
      file: required("file"),
      plutil: required("plutil"),
      spctl: required("spctl"),
      xcrun: required("xcrun"),
    },
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const evidence = verifyReleaseCandidate(args);
  process.stdout.write(`OOMU_RELEASE_CANDIDATE_EVIDENCE=${resolve(args.outputPath)}\n`);
  process.stdout.write(`OOMU_RELEASE_CANDIDATE_TREE_DIGEST=${evidence.application.treeDigest}\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU RELEASE CANDIDATE INTEGRITY FAILED: ${error.message}`);
    process.exit(1);
  }
}
