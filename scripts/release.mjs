#!/usr/bin/env node

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign as signPayload,
} from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  accessSync,
  chmodSync,
  constants,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import {
  materializeCanonicalReleaseEvidence as materializeCanonicalEvidenceRecords,
} from "./release-canonical-evidence.mjs";
import {
  artifactDigestForEntries,
  collectTreeEntries,
  TRUSTED_RELEASE_PUBLIC_KEY_HEX,
} from "./release-manifest.mjs";
import { verifyReleaseAuthorization } from "./assert-release-entrypoint.mjs";
import {
  assertToolUnchanged,
  collectReleaseToolchain,
} from "./release-provenance.mjs";
import {
  assertNoReleaseEnvironmentOverrides,
  assertNoRepositoryDotenvFiles as assertNoRepositoryDotenvFilesAt,
  canonicalNativePathRemapEnvironment,
  createSanitizedChildEnvironment,
  externalHarnessEnvironment,
  releaseToolchainHomeDirectory,
} from "./release-environment.mjs";
import { runCleanMachineQualification } from "./release-clean-machine.mjs";
import {
  assertDragInstallDmgRoot,
  stageApplicationsShortcut,
} from "./release-dmg-layout.mjs";
import {
  loadReleaseVersionRecord,
  releaseArtifactIdentifier,
  releaseDmgName,
} from "./release-version.mjs";
import { writeCanonicalSignedCandidateDescriptor } from "./release-candidate-descriptor.mjs";
import { normalizeUpdaterPublicKey } from "./updater-signature-verification.mjs";
import {
  assertSignedArtifactUnchanged,
  orderedNestedCodeTargets,
  signedArtifactIdentity,
} from "./release-signing-order.mjs";

export { assertNoReleaseEnvironmentOverrides, externalHarnessEnvironment };

export function detachDmgFromCreationHelper(dmgPath) {
  const detachedPath = `${dmgPath}.detached-${process.pid}`;
  const originalDigest = sha256(readFileSync(dmgPath));
  try {
    copyFileSync(dmgPath, detachedPath, constants.COPYFILE_EXCL);
    const detachedDigest = sha256(readFileSync(detachedPath));
    if (detachedDigest !== originalDigest) {
      throw new Error("dmg_detach_digest_mismatch: copied DMG bytes changed");
    }
    renameSync(detachedPath, dmgPath);
  } finally {
    rmSync(detachedPath, { force: true });
  }
  return originalDigest;
}

const root = resolve(import.meta.dirname, "..");
const tauriConfig = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const releaseVersion = loadReleaseVersionRecord(root);
const captures = new Map();
const approvedRepositoryRunners = new Map();
let immutableReleaseToolchain = null;
const EXPECTED_RELEASE_ARCHITECTURE = "arm64";
const EXPECTED_RELEASE_TARGET = "aarch64-apple-darwin";
const NATIVE_INTEGRATION_TESTS = [
  "background_runtime_profile",
  "build_identity_policy",
  "capability_parity",
  "drag_drop_runtime_contract",
  "mcp_security_tests",
  "p1_contracts",
  "pdf_containment",
  "teardown_tests",
  "workflow_jail_tests",
  "workflow_security_tests",
];
const PERMISSION_LINEAGE_BOOTSTRAP_SHA256 =
  "d234d1df1ce3898cf4c3195227fab4a04a879f2feb7ab1be4a6be8d5270115da";
const LOCAL_GATE_LABELS = [
  "automated_strict_lint",
  "automated_version",
  "automated_source_size",
  "automated_real_components",
  "automated_p0_architecture",
  "automated_p1_contracts",
  "automated_novice_ui",
  "automated_module_cycles",
  "automated_unused_exports",
  "automated_repository_hygiene",
  "automated_native_path_remap",
  "automated_release_integrity",
  "automated_i18n",
  "automated_typecheck",
  "automated_frontend",
  "automated_cargo_check",
  "automated_cargo_test",
  "automated_cargo_test_artifacts",
  "automated_cargo_test_integrations",
  "automated_cargo_test_docs",
];

const SIGNING_PREFLIGHT_ENV = [
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_NOTARY_KEYCHAIN_PROFILE",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
];

export function assertNoRepositoryDotenvFiles(checkoutRoot = root) {
  return assertNoRepositoryDotenvFilesAt(checkoutRoot);
}

export function sanitizedChildEnvironment(overrides = {}, source = process.env) {
  return createSanitizedChildEnvironment(overrides, source, immutableReleaseToolchain);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function output(executable, args, options = {}) {
  const resolvedExecutable = resolveExecutable(executable);
  const result = spawnSync(resolvedExecutable, args, {
    cwd: options.cwd ?? root,
    env: options.env ?? sanitizedChildEnvironment(),
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${basename(resolvedExecutable)} failed: ${(result.stderr || result.stdout || "no output").trim()}`,
    );
  }
  return (result.stdout ?? "").trim();
}

function resolveExecutable(executable) {
  const candidate = executable.startsWith("/") ? realpathSync(executable) : null;
  if (immutableReleaseToolchain) {
    const aliases = {
      node: "node",
      npm: "npm",
      rustup: "rustup",
      cargo: "cargo",
      rustc: "rustc",
      git: "git",
    };
    const namedTool = candidate
      ? Object.values(immutableReleaseToolchain.tools)
        .find((tool) => tool.executable === candidate)
      : immutableReleaseToolchain.tools[aliases[executable]];
    if (namedTool) {
      assertToolUnchanged(namedTool);
      return namedTool.executable;
    }
    if (!candidate) {
      throw new Error(`Release executable '${executable}' is outside the approved toolchain.`);
    }
    const repositoryRunner = approvedRepositoryRunners.get(candidate);
    if (
      !repositoryRunner ||
      sha256(readFileSync(candidate)) !== repositoryRunner.sha256
    ) {
      throw new Error(`Release executable '${executable}' is outside the approved toolchain.`);
    }
    return candidate;
  }
  if (candidate) return candidate;
  const probe = spawnSync("/usr/bin/which", [executable], {
    cwd: root,
    env: sanitizedChildEnvironment(),
    encoding: "utf8",
  });
  if (probe.error) throw probe.error;
  if (probe.status !== 0 || !probe.stdout.trim()) {
    throw new Error(`Release executable '${executable}' is unavailable.`);
  }
  return realpathSync(probe.stdout.trim());
}

function runStep(label, executable, args, options = {}) {
  if (captures.has(label)) throw new Error(`Duplicate release step label: ${label}`);
  const resolvedExecutable = resolveExecutable(executable);
  const started = new Date();
  console.log(`[release] ${label}`);
  const result = spawnSync(resolvedExecutable, args, {
    cwd: options.cwd ?? root,
    env: options.env ?? sanitizedChildEnvironment(),
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
  const finished = new Date();
  if (result.error) throw result.error;
  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  if (options.echo !== false) {
    if (stdout.trim()) process.stdout.write(stdout.endsWith("\n") ? stdout : `${stdout}\n`);
    if (stderr.trim()) process.stderr.write(stderr.endsWith("\n") ? stderr : `${stderr}\n`);
  }
  const execution = {
    executed: true,
    exit_code: result.status ?? -1,
    started_at: started.toISOString(),
    finished_at: finished.toISOString(),
    executable: resolvedExecutable,
    arguments: options.evidenceArgs ?? args,
    stdout_sha256: sha256(stdout),
    stderr_sha256: sha256(stderr),
  };
  captures.set(label, { execution, stdout, stderr });
  if (result.status !== 0) {
    const detail = options.suppressFailureOutput
      ? "output withheld"
      : (stderr || stdout || "no output").trim();
    throw new Error(
      `${label} failed with exit ${result.status}: ${detail}`,
    );
  }
  return captures.get(label);
}

function combinedExecution(labels, executable) {
  const executions = labels.map((label) => captures.get(label)?.execution);
  if (executions.some((execution) => !execution || execution.exit_code !== 0)) {
    throw new Error(`Cannot combine incomplete evidence: ${labels.join(", ")}`);
  }
  return {
    executed: true,
    exit_code: 0,
    started_at: executions[0].started_at,
    finished_at: executions.at(-1).finished_at,
    executable: resolveExecutable(executable),
    arguments: labels.map((label) => `[executed:${label}]`),
    subcommands: executions,
  };
}

function parsePlatform(argv) {
  if (argv.length === 0) return "macos";
  if (argv.length === 2 && argv[0] === "--platform") return argv[1];
  throw new Error("Usage: release.mjs [--platform macos]");
}

export function assertExpectedReleaseArchitecture(
  architectureOutput,
  expected = EXPECTED_RELEASE_ARCHITECTURE,
) {
  const architectures = String(architectureOutput)
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (architectures.length !== 1 || architectures[0] !== expected) {
    throw new Error(
      `Staged main executable architecture must be exactly ${expected}; found ${architectures.join(" ") || "none"}.`,
    );
  }
  return architectures[0];
}

function requireEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required by the canonical release pipeline.`);
  return value;
}

export function validateUpdaterPublicKey(value) {
  return normalizeUpdaterPublicKey(value);
}

function repositoryReleaseRunner(fileName) {
  const allowed = new Set([
    "clean-machine-launch.mjs",
    "p0-acceptance.mjs",
  ]);
  if (!allowed.has(fileName)) throw new Error("Release runner is not reviewed.");
  const runner = realpathSync(join(root, "scripts", "release-runners", fileName));
  accessSync(runner, constants.X_OK);
  approvedRepositoryRunners.set(runner, {
    sha256: sha256(readFileSync(runner)),
  });
  return runner;
}

export function createReleaseAuthorization(
  privateKeyPath,
  buildId,
  sourceRevision,
  trustedPublicKeyHex = TRUSTED_RELEASE_PUBLIC_KEY_HEX,
) {
  const privateKey = createPrivateKey(readFileSync(privateKeyPath));
  if (privateKey.asymmetricKeyType !== "ed25519") {
    throw new Error("Canonical release authorization requires an Ed25519 private key.");
  }
  const publicKey = createPublicKey(privateKey);
  const rawPublicKey = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  if (rawPublicKey.toString("hex") !== trustedPublicKeyHex) {
    throw new Error("Release authorization key does not match the reviewed trust root.");
  }
  const payload = `oomu-release-v1\n${buildId}\n${sourceRevision}`;
  return signPayload(null, Buffer.from(payload, "utf8"), privateKey).toString("base64");
}

function assertCleanSourceTree() {
  const status = output("git", ["status", "--porcelain=v1", "--untracked-files=all"]);
  if (status) {
    throw new Error(`The audited source tree is not clean:\n${status}`);
  }
}

export function createExclusiveReleaseOutputDirectories(evidenceDir, candidateDir) {
  const evidence = resolve(evidenceDir);
  const candidate = resolve(candidateDir);
  mkdirSync(dirname(evidence), { recursive: true });
  mkdirSync(dirname(candidate), { recursive: true });
  mkdirSync(evidence, { recursive: false });
  try {
    mkdirSync(candidate, { recursive: false });
  } catch (error) {
    // The evidence directory was created by this invocation and is still
    // empty. Roll it back if the candidate directory loses an exclusivity
    // race, while never touching a pre-existing release output.
    rmSync(evidence, { recursive: true, force: true });
    throw error;
  }
}

function walk(rootPath) {
  const entries = [];
  function visit(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) visit(child);
      else if (entry.isFile()) entries.push(child);
    }
  }
  visit(rootPath);
  return entries;
}

function makeTreeImmutable(rootPath) {
  function visit(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        visit(child);
        chmodSync(child, statSync(child).mode & 0o555);
      } else if (entry.isFile()) {
        const mode = statSync(child).mode;
        chmodSync(child, mode & 0o111 ? 0o555 : 0o444);
      }
    }
  }
  visit(rootPath);
  chmodSync(rootPath, 0o555);
}

function findApp(bundleRoot) {
  const apps = [];
  function visit(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isDirectory() && entry.name.endsWith(".app")) apps.push(child);
      else if (entry.isDirectory()) visit(child);
    }
  }
  visit(bundleRoot);
  if (apps.length !== 1) throw new Error(`Expected one unsigned app bundle; found ${apps.length}.`);
  return apps[0];
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`);
  }
}

function validateAssetCatalog(appPath, toolchain) {
  const assets = walk(appPath).filter((path) => basename(path) === "Assets.car");
  if (assets.length !== 1 || statSync(assets[0]).size === 0) {
    throw new Error(`Expected one non-empty Assets.car; found ${assets.length}.`);
  }
  const result = runStep("asset_catalog_validation", toolchain.tools.assetutil.executable, [
    "--info",
    assets[0],
  ]);
  let catalog;
  try {
    catalog = JSON.parse(result.stdout);
  } catch {
    throw new Error("assetutil returned malformed asset-catalog output.");
  }
  if (!Array.isArray(catalog) || catalog.length === 0) {
    throw new Error("Assets.car has no structurally valid asset entries.");
  }
  return { path: assets[0], size_bytes: statSync(assets[0]).size, asset_count: catalog.length };
}

function prepareGeneratedApplicationForSigning(appPath, toolchain) {
  const assetResult = validateAssetCatalog(appPath, toolchain);
  runStep("clear_generated_bundle_extended_attributes", toolchain.tools.xattr.executable, [
    "-cr", appPath,
  ]);
  return assetResult;
}

function codesignArguments(identity, path, entitlements = null, hardenedRuntime = true) {
  const args = ["--force", "--timestamp"];
  if (hardenedRuntime) args.push("--options", "runtime");
  args.push("--sign", identity);
  if (entitlements) args.push("--entitlements", entitlements);
  args.push(path);
  return args;
}

function signNestedCode(appPath, identity, codesign) {
  const targets = orderedNestedCodeTargets(appPath, "/usr/bin/file");
  const executions = [];
  for (const [index, target] of targets.entries()) {
    const label = `codesign_nested_${index}`;
    runStep(label, codesign, codesignArguments(identity, target), {
      echo: false,
      evidenceArgs: [
        "--force", "--timestamp", "--options", "runtime", "--sign", "<reviewed-identity>",
        relative(appPath, target),
      ],
    });
    executions.push(label);
  }
  return executions;
}

function writeArtifactHelperIntegrityManifest(appPath) {
  const helpers = {};
  for (const name of ["artifact_build_helper", "oomu-artifact-pdf-helper"]) {
    const path = join(appPath, "Contents", "MacOS", name);
    if (!existsSync(path) || !statSync(path).isFile()) {
      throw new Error(`Required artifact helper is missing: ${name}`);
    }
    helpers[name] = sha256(readFileSync(path));
  }
  const destination = join(
    appPath,
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

function notarizationArgs(path, credentials) {
  const args = ["notarytool", "submit", path, "--wait", "--output-format", "json"];
  const redacted = ["notarytool", "submit", basename(path), "--wait", "--output-format", "json"];
  if (credentials.mode === "keychain-profile") {
    args.push("--keychain-profile", credentials.profile);
    redacted.push("--keychain-profile", credentials.profile);
  } else if (credentials.mode === "apple-id") {
    args.push("--apple-id", credentials.appleId, "--password", credentials.password, "--team-id", credentials.teamId);
    redacted.push("--apple-id", "<redacted>", "--password", "<redacted>", "--team-id", credentials.teamId);
  } else {
    args.push("--issuer", credentials.issuer, "--key-id", credentials.keyId, "--key", credentials.keyPath);
    redacted.push("--issuer", "<redacted>", "--key-id", credentials.keyId, "--key", basename(credentials.keyPath));
  }
  return { args, redacted };
}

function parseNotaryResult(capture, label) {
  const result = JSON.parse(capture.stdout);
  if (result.status !== "Accepted" || !result.id) {
    throw new Error(`${label} was not accepted by Apple notarization.`);
  }
  return { id: result.id, status: result.status };
}

function signingDetails(codesign, appPath, expectedTeamId, expectedBundleIdentifier) {
  const capture = runStep("codesign_details", codesign, ["-d", "--verbose=4", appPath], {
    echo: false,
  });
  const details = `${capture.stdout}\n${capture.stderr}`;
  const teamId = details.match(/^TeamIdentifier=(.+)$/m)?.[1]?.trim();
  const bundleIdentifier = details.match(/^Identifier=(.+)$/m)?.[1]?.trim();
  const authority = details.match(/^Authority=(.+)$/m)?.[1]?.trim();
  const hardenedRuntime = /flags=.*\(runtime\)/.test(details);
  if (
    teamId !== expectedTeamId ||
    bundleIdentifier !== expectedBundleIdentifier ||
    !authority?.startsWith("Developer ID Application:") ||
    !hardenedRuntime
  ) {
    throw new Error(
      "Signed app does not have the expected production identifier, Team ID, Developer ID authority, and hardened runtime.",
    );
  }
  return {
    bundle_identifier: bundleIdentifier,
    team_id: teamId,
    authority,
    hardened_runtime: true,
  };
}

function materializeCanonicalReleaseEvidence(context) {
  return materializeCanonicalEvidenceRecords(context, {
    captures,
    combinedExecution,
    readJson,
    resolveExecutable,
    expectedReleaseTarget: EXPECTED_RELEASE_TARGET,
  });
}

function runRustQualification(node, releaseEnvironment) {
  runStep("external_bin_manifest_reservation", node, ["scripts/prepare-tauri-external-bins.mjs"], { env: releaseEnvironment });
  runStep("portable_python_preparation", node, ["scripts/prepare-portable-python.mjs", "--release"], { env: releaseEnvironment });
  const cargo = resolveExecutable("cargo");
  const rustDependencyAudit = runStep("rust_dependency_audit", node, ["scripts/audit-rust-dependencies.mjs"], { echo: false, env: releaseEnvironment });
  const rustDependencyAuditResult = JSON.parse(rustDependencyAudit.stdout);
  const pdfContainment = runStep("pdf_containment", cargo, [
    "test", "--manifest-path", "src-tauri/Cargo.toml", "--test", "pdf_containment",
    "real_corpus_processes_are_bounded_and_deterministic", "--", "--exact", "--nocapture",
  ], { env: releaseEnvironment });
  const pdfEvidenceLine = pdfContainment.stdout
    .split(/\r?\n/u)
    .find((line) => line.startsWith("PDF_CONTAINMENT_EVIDENCE_JSON="));
  if (!pdfEvidenceLine) throw new Error("PDF containment corpus did not emit executed evidence.");
  const pdfContainmentResult = JSON.parse(
    pdfEvidenceLine.slice("PDF_CONTAINMENT_EVIDENCE_JSON=".length),
  );
  runStep("automated_cargo_check", cargo, [
    "check", "--locked", "--target", EXPECTED_RELEASE_TARGET,
    "--manifest-path", "src-tauri/Cargo.toml",
  ], { env: releaseEnvironment });
  runStep("automated_cargo_test", cargo, [
    "test", "--locked", "--target", EXPECTED_RELEASE_TARGET,
    "--manifest-path", "src-tauri/Cargo.toml",
    "--lib", "--", "--test-threads=10", "--skip", "artifacts::",
  ], { env: releaseEnvironment });
  runStep("automated_cargo_test_artifacts", cargo, [
    "test", "--locked", "--target", EXPECTED_RELEASE_TARGET,
    "--manifest-path", "src-tauri/Cargo.toml",
    "--lib", "artifacts::", "--", "--test-threads=1",
  ], { env: releaseEnvironment });
  runStep("automated_cargo_test_integrations", cargo, [
    "test", "--locked", "--target", EXPECTED_RELEASE_TARGET,
    "--manifest-path", "src-tauri/Cargo.toml",
    ...NATIVE_INTEGRATION_TESTS.flatMap((name) => ["--test", name]),
    "--", "--test-threads=10",
  ], { env: releaseEnvironment });
  runStep("automated_cargo_test_docs", cargo, [
    "test", "--locked", "--target", EXPECTED_RELEASE_TARGET,
    "--manifest-path", "src-tauri/Cargo.toml", "--doc",
  ], { env: releaseEnvironment });
  return { pdfContainmentResult, rustDependencyAuditResult };
}

export function permissionContinuityPrerequisite(
  environment = process.env,
  options = {},
) {
  if (environment.OOMU_FIRST_SIGNED_RELEASE?.trim()) {
    throw new Error(
      "OOMU_FIRST_SIGNED_RELEASE is no longer accepted. First-release status comes from the reviewed permission-lineage record.",
    );
  }
  const configuredPreviousApp = environment.OOMU_PREVIOUS_SIGNED_APP?.trim();
  const previousSignedAppInput = resolve(
    configuredPreviousApp || options.defaultPreviousApp || "/Applications/OOMU.app",
  );
  if (configuredPreviousApp && !existsSync(previousSignedAppInput)) {
    throw new Error(
      "OOMU_PREVIOUS_SIGNED_APP does not point to an existing signed app.",
    );
  }
  const previousSignedApp = existsSync(previousSignedAppInput)
    ? realpathSync(previousSignedAppInput)
    : null;
  if (
    previousSignedApp
    && (!statSync(previousSignedApp).isDirectory() || !previousSignedApp.endsWith(".app"))
  ) {
    throw new Error("The previous signed OOMU release must be an existing .app bundle.");
  }
  if (previousSignedApp) {
    return { firstSignedRelease: false, previousSignedApp, lineageEvidencePath: null };
  }

  const lineageEvidencePath = resolve(
    options.lineageEvidencePath
      || join(root, "release", "macos-permission-lineage.json"),
  );
  if (!existsSync(lineageEvidencePath) || !statSync(lineageEvidencePath).isFile()) {
    throw new Error(
      "A previous signed OOMU app is required because the reviewed first-release permission lineage is unavailable.",
    );
  }
  const lineageBytes = readFileSync(lineageEvidencePath);
  if (sha256(lineageBytes) !== PERMISSION_LINEAGE_BOOTSTRAP_SHA256) {
    throw new Error("The reviewed first-release permission lineage changed.");
  }
  let lineage;
  try {
    lineage = JSON.parse(lineageBytes.toString("utf8"));
  } catch {
    throw new Error("The reviewed first-release permission lineage is not valid JSON.");
  }
  const expectedKeys = [
    "bundleIdentifier",
    "firstBuildNumber",
    "firstProductVersion",
    "kind",
    "lineageId",
    "schemaVersion",
  ];
  if (JSON.stringify(Object.keys(lineage).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("The reviewed first-release permission lineage has unexpected fields.");
  }
  const currentRelease = options.releaseVersion ?? releaseVersion;
  const bundleIdentifier = options.bundleIdentifier ?? tauriConfig.identifier;
  if (
    lineage.schemaVersion !== 1
    || lineage.kind !== "oomu.macos-permission-lineage.bootstrap"
    || lineage.lineageId !== `oomu-public-macos-${bundleIdentifier}`
    || lineage.bundleIdentifier !== bundleIdentifier
    || lineage.firstProductVersion !== currentRelease.productVersion
    || lineage.firstBuildNumber !== currentRelease.buildNumber
  ) {
    throw new Error(
      "This is not the reviewed first signed release. Provide the previous signed OOMU app for permission continuity.",
    );
  }
  return { firstSignedRelease: true, previousSignedApp: null, lineageEvidencePath };
}

function capturePreviousPermissionIdentity({
  node,
  previousSignedApp,
  rawEvidenceDir,
  teamId,
  lineageEvidencePath,
}) {
  const outputPath = join(rawEvidenceDir, "previous-signed-permission-identity.json");
  if (lineageEvidencePath) {
    copyFileSync(
      lineageEvidencePath,
      join(rawEvidenceDir, "first-signed-release-permission-lineage.json"),
      constants.COPYFILE_EXCL,
    );
  }
  if (previousSignedApp) {
    runStep("permission_identity_previous", node, [
      "scripts/release-gates/macos-permission-continuity.mjs",
      "--snapshot-app", previousSignedApp,
      "--output", outputPath,
      "--expected-team", teamId,
    ]);
  }
  return outputPath;
}

function verifyPermissionContinuity({
  firstSignedRelease,
  node,
  previousPermissionIdentityPath,
  rawEvidenceDir,
  stagedApp,
  teamId,
}) {
  const outputPath = join(rawEvidenceDir, "macos-permission-continuity.json");
  const argumentsForGate = firstSignedRelease
    ? [
        "--snapshot-app", stagedApp,
        "--output", outputPath,
        "--expected-team", teamId,
        "--first-signed-release",
      ]
    : [
        "--signed-app", stagedApp,
        "--previous", previousPermissionIdentityPath,
        "--output", outputPath,
        "--expected-team", teamId,
      ];
  runStep(
    firstSignedRelease ? "permission_identity_current" : "permission_continuity",
    node,
    ["scripts/release-gates/macos-permission-continuity.mjs", ...argumentsForGate],
  );
  return readJson(outputPath, "macOS permission continuity report");
}

function captureSourceAndDependencyInputs(sourceRevision) {
  const revisionCapture = runStep("source_revision", "git", ["rev-parse", "HEAD"], {
    echo: false,
  });
  const dirtyCapture = runStep(
    "source_dirty_state",
    "git",
    ["status", "--porcelain=v1", "--untracked-files=all"],
    { echo: false },
  );
  if (
    revisionCapture.stdout.trim() !== sourceRevision
    || dirtyCapture.stdout.trim() !== ""
  ) {
    throw new Error("Source provenance changed after release authorization.");
  }
  return {
    package_lock_sha256: sha256(readFileSync(join(root, "package-lock.json"))),
    cargo_lock_sha256: sha256(readFileSync(join(root, "src-tauri", "Cargo.lock"))),
    source_line_baseline_sha256: sha256(
      readFileSync(join(root, "scripts", "source-line-baselines.tsv")),
    ),
  };
}

function initializeReleaseContext() {
  const platform = parsePlatform(process.argv.slice(2));
  if (platform !== "macos" || process.platform !== "darwin") {
    throw new Error(
      "The canonical distributable release currently supports only signed/notarized macOS candidates.",
    );
  }
  if (process.arch !== EXPECTED_RELEASE_ARCHITECTURE) {
    throw new Error(
      `The canonical release runner must execute natively as ${EXPECTED_RELEASE_ARCHITECTURE}; found ${process.arch}.`,
    );
  }
  assertNoRepositoryDotenvFiles();
  assertNoReleaseEnvironmentOverrides();
  immutableReleaseToolchain = collectReleaseToolchain({
    ciPhase: false,
    protectedPhase: false,
  });
  assertCleanSourceTree();
  const sourceRevision = output("git", ["rev-parse", "HEAD"]);
  if (!/^[0-9a-f]{40}$/iu.test(sourceRevision)) {
    throw new Error("Unable to resolve a full source revision.");
  }
  const buildId = process.env.OOMU_BUILD_ID?.trim()
    || `${sourceRevision.slice(0, 12)}-${new Date().toISOString().replace(/[-:.TZ]/gu, "")}`;
  if (!/^[A-Za-z0-9._-]{8,128}$/u.test(buildId)) {
    throw new Error("OOMU_BUILD_ID has unsafe characters.");
  }
  const artifactId = releaseArtifactIdentifier(releaseVersion, buildId);
  const privateKeyPath = resolve(requireEnvironment("OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH"));
  const publicKeyPath = resolve(requireEnvironment("OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH"));
  const signingIdentity = requireEnvironment("APPLE_SIGNING_IDENTITY");
  const teamId = requireEnvironment("APPLE_TEAM_ID");
  const updaterPublicKey = validateUpdaterPublicKey(
    requireEnvironment("OOMU_UPDATER_PUBLIC_KEY"),
  );
  const permission = permissionContinuityPrerequisite();
  const cleanMachineRunner = repositoryReleaseRunner("clean-machine-launch.mjs");
  const p0AcceptanceRunner = repositoryReleaseRunner("p0-acceptance.mjs");
  const oauthSecretScanCanaries = requireEnvironment(
    "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
  );
  const releaseAuthorization = createReleaseAuthorization(
    privateKeyPath, buildId, sourceRevision,
  );
  if (!verifyReleaseAuthorization({
    buildId,
    sourceRevision,
    signatureBase64: releaseAuthorization,
    publicKeyPath,
  })) {
    throw new Error("Release public key does not match the reviewed authorization key.");
  }
  const releaseEnvironment = {
    ...sanitizedChildEnvironment({
      OOMU_RELEASE_PIPELINE: "canonical-v1",
      OOMU_BUILD_ID: buildId,
      OOMU_SOURCE_REVISION: sourceRevision,
      OOMU_RELEASE_AUTHORIZATION_BASE64: releaseAuthorization,
      OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH: publicKeyPath,
      OOMU_UPDATER_PUBLIC_KEY: updaterPublicKey,
    }),
    ...canonicalNativePathRemapEnvironment(
      root, process.env,
      releaseToolchainHomeDirectory(immutableReleaseToolchain),
    ),
  };
  const signingPreflightEnvironment = sanitizedChildEnvironment(Object.fromEntries(
    SIGNING_PREFLIGHT_ENV
      .filter((name) => process.env[name] !== undefined)
      .map((name) => [name, process.env[name]]),
  ));
  const credentials = process.env.APPLE_NOTARY_KEYCHAIN_PROFILE?.trim()
    ? {
        mode: "keychain-profile",
        profile: process.env.APPLE_NOTARY_KEYCHAIN_PROFILE.trim(),
      }
    : process.env.APPLE_ID && process.env.APPLE_PASSWORD
      ? {
        mode: "apple-id",
        appleId: process.env.APPLE_ID,
        password: process.env.APPLE_PASSWORD,
        teamId,
      }
      : {
        mode: "api-key",
        issuer: requireEnvironment("APPLE_API_ISSUER"),
        keyId: requireEnvironment("APPLE_API_KEY"),
        keyPath: resolve(requireEnvironment("APPLE_API_KEY_PATH")),
      };
  const workDir = mkdtempSync(join(tmpdir(), `oomu-release-${buildId}-`));
  const rawEvidenceDir = join(workDir, "raw-evidence");
  const evidenceDir = join(root, "release", "evidence", buildId);
  const candidateDir = join(root, "release", "candidates", buildId);
  if (existsSync(evidenceDir) || existsSync(candidateDir)) {
    throw new Error(
      "Build identifier already has release output; evidence is immutable and cannot be reused.",
    );
  }
  mkdirSync(rawEvidenceDir, { recursive: true });
  createExclusiveReleaseOutputDirectories(evidenceDir, candidateDir);
  const node = process.execPath;
  const npm = resolveExecutable("npm");
  const previousPermissionIdentityPath = capturePreviousPermissionIdentity({
    node,
    previousSignedApp: permission.previousSignedApp,
    rawEvidenceDir,
    teamId,
    lineageEvidencePath: permission.lineageEvidencePath,
  });
  return {
    sourceRevision, buildId, artifactId, privateKeyPath, publicKeyPath,
    signingIdentity, teamId, cleanMachineRunner, p0AcceptanceRunner,
    oauthSecretScanCanaries, releaseEnvironment, signingPreflightEnvironment,
    credentials, workDir, rawEvidenceDir, evidenceDir, candidateDir, node, npm,
    previousPermissionIdentityPath,
    firstSignedRelease: permission.firstSignedRelease,
    dependencyLockDigests: captureSourceAndDependencyInputs(sourceRevision),
  };
}

function runAutomatedReleaseGates(context) {
  const { node, npm, rawEvidenceDir, signingPreflightEnvironment, releaseEnvironment } = context;
  const automatedEnvironment = sanitizedChildEnvironment();
  runStep("apple_toolchain", node, [
    "scripts/preflight-apple-toolchain.mjs",
    "--output",
    join(rawEvidenceDir, "toolchain.json"),
  ]);
  const toolchain = readJson(join(rawEvidenceDir, "toolchain.json"), "Apple toolchain report");
  runStep("entitlement_snapshot", node, ["scripts/check-entitlements.mjs"]);
  runStep("signing_preflight", node, ["scripts/preflight_signing.js"], {
    env: signingPreflightEnvironment,
  });
  const dependencyAudit = runStep(
    "dependency_audit",
    npm,
    ["audit", "--omit=dev", "--audit-level=high", "--json"],
    { echo: false },
  );
  const dependencyResult = JSON.parse(dependencyAudit.stdout);
  const nativeCompiler = immutableReleaseToolchain.tools.clang;
  const nativeCargo = immutableReleaseToolchain.tools.cargo;
  const nativePathRemapPath = join(rawEvidenceDir, "native-path-remap-preflight.json");
  runStep("automated_native_path_remap", node, [
    "scripts/preflight-native-path-remap.mjs",
    "--cargo", nativeCargo.executable,
    "--clang", nativeCompiler.executable,
    "--output", nativePathRemapPath,
  ], { env: releaseEnvironment });
  const nativePathRemap = readJson(nativePathRemapPath, "Native path-remap preflight");
  if (nativePathRemap.schema_version !== 1
    || nativePathRemap.kind !== "oomu.native-path-remap-preflight"
    || nativePathRemap.status !== "passed" || nativePathRemap.synthetic !== false
    || nativePathRemap.compiler_sha256 !== nativeCompiler.sha256
    || nativePathRemap.cargo_sha256 !== nativeCargo.sha256
    || nativePathRemap.release_cache?.status !== "passed"
    || nativePathRemap.release_cache?.remaining_stale_cache_count !== 0
    || !Array.isArray(nativePathRemap.checked_languages)
    || JSON.stringify(nativePathRemap.checked_languages.map((entry) => entry.language))
      !== JSON.stringify(["c", "c++"])
    || nativePathRemap.checked_languages.some((entry) =>
      entry.local_path_findings.length !== 0
      || !/^[a-f0-9]{64}$/u.test(entry.object_sha256)
      || !/^[a-f0-9]{64}$/u.test(entry.canonical_path_sha256))) {
    throw new Error("Native path-remap preflight did not produce bound passing evidence.");
  }
  runStep("automated_strict_lint", npm, ["run", "lint"], { env: automatedEnvironment });
  runStep("automated_version", npm, ["run", "check:version"], { env: automatedEnvironment });
  runStep("automated_source_size", npm, ["run", "check:source-size"], { env: automatedEnvironment });
  runStep("automated_real_components", npm, ["run", "check:real-components"], { env: automatedEnvironment });
  runStep("automated_p0_architecture", npm, ["run", "check:p0-architecture"], { env: automatedEnvironment });
  runStep("automated_p1_contracts", npm, ["run", "check:p1-contracts"], { env: automatedEnvironment });
  runStep("automated_novice_ui", npm, ["run", "check:novice-ui"], { env: automatedEnvironment });
  runStep("automated_module_cycles", npm, ["run", "check:module-cycles"], { env: automatedEnvironment });
  runStep("automated_unused_exports", npm, ["run", "check:unused-exports"], { env: automatedEnvironment });
  runStep("automated_repository_hygiene", npm, ["run", "check:repository-hygiene"], { env: automatedEnvironment });
  runStep("automated_release_integrity", npm, ["run", "test:release-integrity"], { env: automatedEnvironment });
  runStep("automated_i18n", npm, ["run", "check:i18n"], { env: automatedEnvironment });
  runStep("automated_typecheck", npm, ["run", "typecheck"], { env: automatedEnvironment });
  runStep("automated_frontend", npm, ["run", "test:frontend"], { env: automatedEnvironment });
  return {
    toolchain,
    dependencyResult,
    nativePathRemap,
    ...runRustQualification(node, releaseEnvironment),
  };
}

function buildAndSignApplication(context, toolchain) {
  const { node, npm, rawEvidenceDir, releaseEnvironment, buildId, signingIdentity, teamId } =
    context;
  const bundleRoot = join(root, "src-tauri", "target", "release", "bundle");
  rmSync(bundleRoot, { recursive: true, force: true });
  runStep("compile", npm, [
    "run", "tauri:build:internal", "--", "--config",
    "src-tauri/tauri.release.conf.json", "--no-bundle",
  ], { env: releaseEnvironment });
  runStep("unsigned_bundle", npm, [
    "run", "tauri:bundle:internal", "--", "--config",
    "src-tauri/tauri.release.conf.json", "--bundles", "app", "--no-sign",
  ], { env: releaseEnvironment });
  assertCleanSourceTree();
  const appPath = findApp(bundleRoot);
  const sanitizerInitialPath = join(rawEvidenceDir, "release-sanitizer-initial.json");
  runStep("release_sanitizer_initial", node, [
    "scripts/sanitize-release.mjs", "--dir", appPath, "--execute",
    "--build-id", buildId, "--evidence", sanitizerInitialPath,
  ]);
  const databaseInitialPath = join(rawEvidenceDir, "database-sanitizer-initial.json");
  runStep("database_sanitizer_initial", node, [
    "scripts/sanitize-release-db.mjs", "--release", "--dir", appPath,
    "--build-id", buildId, "--evidence", databaseInitialPath,
  ], { env: releaseEnvironment });
  const assetResult = prepareGeneratedApplicationForSigning(appPath, toolchain);
  const unsignedBuildPathPrivacyPath = join(
    rawEvidenceDir, "unsigned-build-path-privacy.json",
  );
  runStep("unsigned_build_path_privacy", node, [
    "scripts/check-build-path-privacy.mjs",
    "--app", appPath,
    "--output", unsignedBuildPathPrivacyPath,
  ]);
  const codesign = toolchain.tools.codesign.executable;
  const nestedSigningLabels = signNestedCode(appPath, signingIdentity, codesign);
  writeArtifactHelperIntegrityManifest(appPath);
  const sanitizerRawPath = join(rawEvidenceDir, "release-sanitizer-final.json");
  runStep("release_sanitizer_final", node, [
    "scripts/sanitize-release.mjs", "--dir", appPath, "--execute",
    "--build-id", buildId, "--evidence", sanitizerRawPath,
  ]);
  const databaseRawPath = join(rawEvidenceDir, "database-sanitizer-final.json");
  runStep("database_sanitizer_final", node, [
    "scripts/sanitize-release-db.mjs", "--release", "--dir", appPath,
    "--build-id", buildId, "--evidence", databaseRawPath,
  ], { env: releaseEnvironment });
  runStep("codesign_app", codesign, codesignArguments(
    signingIdentity,
    appPath,
    join(root, "src-tauri", "entitlements.plist"),
  ), {
    evidenceArgs: [
      "--force", "--timestamp", "--options", "runtime", "--sign",
      "<reviewed-identity>", "--entitlements", "src-tauri/entitlements.plist", basename(appPath),
    ],
  });
  runStep("codesign_verify_app", codesign, [
    "--verify", "--deep", "--strict", "--verbose=4", appPath,
  ]);
  const signedApplicationIdentity = signedArtifactIdentity(appPath);
  return {
    appPath,
    sanitizerInitialPath,
    databaseInitialPath,
    assetResult,
    codesign,
    nestedSigningLabels,
    sanitizerRawPath,
    databaseRawPath,
    signedApplicationIdentity,
    signedDetails: signingDetails(codesign, appPath, teamId, tauriConfig.identifier),
  };
}

function notarizeAndCreateDmg(context, toolchain, built) {
  const { workDir, credentials, signingIdentity } = context;
  const { appPath, codesign } = built;
  const appZip = join(workDir, `${tauriConfig.productName}.zip`);
  assertSignedArtifactUnchanged(
    appPath, built.signedApplicationIdentity, "before_app_notarization_archive",
  );
  runStep("archive_app_for_notary", toolchain.tools.ditto.executable, [
    "-c", "-k", "--keepParent", appPath, appZip,
  ]);
  const appNotary = notarizationArgs(appZip, credentials);
  const appNotaryCapture = runStep(
    "notarize_app",
    "/usr/bin/xcrun",
    appNotary.args,
    { evidenceArgs: appNotary.redacted, echo: false },
  );
  const appNotaryResult = parseNotaryResult(appNotaryCapture, "App bundle");
  assertSignedArtifactUnchanged(
    appPath, built.signedApplicationIdentity, "after_app_notarization",
  );
  runStep("staple_app", "/usr/bin/xcrun", ["stapler", "staple", appPath]);
  runStep("validate_staple_app", "/usr/bin/xcrun", ["stapler", "validate", appPath]);
  runStep("codesign_verify_stapled_app", codesign, [
    "--verify", "--deep", "--strict", "--verbose=4", appPath,
  ]);
  const finalApplicationIdentity = signedArtifactIdentity(appPath);
  const dmgSource = join(workDir, "dmg-root");
  mkdirSync(dmgSource, { recursive: true });
  runStep("stage_dmg_app", toolchain.tools.ditto.executable, [
    appPath, join(dmgSource, basename(appPath)),
  ]);
  stageApplicationsShortcut(dmgSource);
  assertDragInstallDmgRoot(dmgSource, appPath);
  const dmgPath = join(
    workDir,
    releaseDmgName(releaseVersion, tauriConfig.productName),
  );
  runStep("create_dmg", toolchain.tools.hdiutil.executable, [
    "create", "-volname", tauriConfig.productName, "-srcfolder", dmgSource,
    "-ov", "-format", "UDZO", dmgPath,
  ]);
  runStep("codesign_dmg", codesign, codesignArguments(
    signingIdentity, dmgPath, null, false,
  ), {
    evidenceArgs: [
      "--force", "--timestamp", "--sign", "<reviewed-identity>", basename(dmgPath),
    ],
  });
  detachDmgFromCreationHelper(dmgPath);
  runStep("verify_dmg_structure", toolchain.tools.hdiutil.executable, [
    "verify", dmgPath,
  ]);
  const dmgNotary = notarizationArgs(dmgPath, credentials);
  const dmgNotaryCapture = runStep(
    "notarize_dmg",
    "/usr/bin/xcrun",
    dmgNotary.args,
    { evidenceArgs: dmgNotary.redacted, echo: false },
  );
  const dmgNotaryResult = parseNotaryResult(dmgNotaryCapture, "DMG");
  runStep("staple_dmg", "/usr/bin/xcrun", ["stapler", "staple", dmgPath]);
  runStep("validate_staple_dmg", "/usr/bin/xcrun", ["stapler", "validate", dmgPath]);
  runStep("gatekeeper_app", toolchain.tools.spctl.executable, [
    "--assess", "--type", "execute", "--verbose=4", appPath,
  ]);
  assertSignedArtifactUnchanged(
    appPath, finalApplicationIdentity, "after_distribution_container_creation",
  );
  return { appNotaryResult, dmgNotaryResult, dmgPath, finalApplicationIdentity };
}

function stageAndVerifyCandidate(context, toolchain, built, notarized) {
  const { candidateDir, rawEvidenceDir, node, teamId, firstSignedRelease,
    previousPermissionIdentityPath, oauthSecretScanCanaries } = context;
  const stagedApp = join(candidateDir, basename(built.appPath));
  assertSignedArtifactUnchanged(
    built.appPath, notarized.finalApplicationIdentity, "before_candidate_staging",
  );
  runStep("stage_app", toolchain.tools.ditto.executable, [built.appPath, stagedApp]);
  assertSignedArtifactUnchanged(
    built.appPath, notarized.finalApplicationIdentity, "after_candidate_staging",
  );
  const stagedDmg = join(candidateDir, basename(notarized.dmgPath));
  copyFileSync(notarized.dmgPath, stagedDmg);
  runStep("verify_staged_codesign", built.codesign, [
    "--verify", "--deep", "--strict", "--verbose=4", stagedApp,
  ]);
  runStep("verify_staged_app_ticket", "/usr/bin/xcrun", [
    "stapler", "validate", stagedApp,
  ]);
  runStep("verify_staged_dmg_ticket", "/usr/bin/xcrun", [
    "stapler", "validate", stagedDmg,
  ]);
  const architectureCapture = runStep(
    "architecture_validation",
    toolchain.tools.lipo.executable,
    ["-archs", join(stagedApp, "Contents", "MacOS", tauriConfig.mainBinaryName)],
  );
  const architecture = assertExpectedReleaseArchitecture(architectureCapture.stdout);
  const finalEntitlementSnapshotPath = join(
    rawEvidenceDir, "final-signed-entitlements.json",
  );
  runStep("final_entitlement_snapshot", node, [
    "scripts/check-entitlements.mjs",
    "--signed-app", stagedApp,
    "--output", finalEntitlementSnapshotPath,
  ]);
  const finalEntitlementSnapshot = readJson(
    finalEntitlementSnapshotPath,
    "Final signed entitlement snapshot",
  );
  const candidateIntegrityPath = join(rawEvidenceDir, "release-candidate-integrity.json");
  runStep("release_candidate_integrity", node, [
    "scripts/release-candidate-integrity.mjs",
    "--app", stagedApp,
    "--container", stagedDmg,
    "--entitlements", finalEntitlementSnapshotPath,
    "--output", candidateIntegrityPath,
    "--team-id", teamId,
    "--bundle-id", tauriConfig.identifier,
    "--build-number", String(releaseVersion.buildNumber),
    "--codesign", toolchain.tools.codesign.executable,
    "--file", toolchain.tools.file.executable,
    "--plutil", toolchain.tools.plutil.executable,
    "--spctl", toolchain.tools.spctl.executable,
    "--xcrun", toolchain.tools.xcrun.executable,
  ]);
  const candidateIntegrity = readJson(
    candidateIntegrityPath,
    "Release candidate integrity report",
  );
  if (candidateIntegrity.kind !== "oomu.release-candidate-integrity"
    || candidateIntegrity.status !== "passed" || candidateIntegrity.synthetic !== false) {
    throw new Error("Release candidate integrity did not produce real passing evidence.");
  }
  const permissionContinuitySnapshot = verifyPermissionContinuity({
    firstSignedRelease,
    node,
    previousPermissionIdentityPath,
    rawEvidenceDir,
    stagedApp,
    teamId,
  });
  const extensionGatesPath = join(rawEvidenceDir, "release-extension-gates.json");
  runStep("release_extension_gates", node, [
    "scripts/run-release-extension-gates.mjs",
    "--app", stagedApp,
    "--evidence-dir", rawEvidenceDir,
    "--toolchain", join(rawEvidenceDir, "toolchain.json"),
    "--output", extensionGatesPath,
  ], {
    env: sanitizedChildEnvironment({
      OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64: oauthSecretScanCanaries,
    }),
    echo: false,
    suppressFailureOutput: true,
  });
  const extensionGates = readJson(extensionGatesPath, "Release extension-gate report");
  if (extensionGates.status !== "passed" || extensionGates.synthetic !== false) {
    throw new Error("Release extension gates did not produce real passing evidence.");
  }
  if (artifactDigestForEntries(collectTreeEntries(stagedApp))
      !== candidateIntegrity.application.treeDigest
    || sha256(readFileSync(stagedDmg)) !== candidateIntegrity.container.sha256) {
    throw new Error(
      "release_candidate_mutated_after_verification: the staged candidate changed after its final integrity check",
    );
  }
  return {
    stagedApp,
    stagedDmg,
    architecture,
    finalEntitlementSnapshot,
    finalEntitlementSnapshotPath,
    candidateIntegrityPath,
    candidateIntegrity,
    permissionContinuitySnapshot,
    extensionGates,
  };
}

function materializeCandidateDescriptor(context, candidate, releaseEvidence) {
  const entitlementReportPath = join(context.evidenceDir, "final-signed-entitlements.json");
  const publicKeyPath = join(context.evidenceDir, "release-manifest-public-key.pem");
  copyFileSync(candidate.finalEntitlementSnapshotPath, entitlementReportPath, constants.COPYFILE_EXCL);
  copyFileSync(context.publicKeyPath, publicKeyPath, constants.COPYFILE_EXCL);
  chmodSync(entitlementReportPath, 0o400);
  chmodSync(publicKeyPath, 0o400);
  return writeCanonicalSignedCandidateDescriptor({
    repositoryRoot: root,
    outputPath: join(context.evidenceDir, "signed-candidate-descriptor.json"),
    input: {
      appPath: candidate.stagedApp,
      containerPath: candidate.stagedDmg,
      entitlementReportPath,
      manifestPath: releaseEvidence.manifestPath,
      publicKeyPath,
      releaseProvenancePath: releaseEvidence.releaseProvenancePath,
      appPrefix: basename(candidate.stagedApp),
      expectedTeamId: context.teamId,
      expectedBundleIdentifier: tauriConfig.identifier,
      expectedBuildNumber: releaseVersion.buildNumber,
      expectedBuildIdentifier: context.buildId,
      expectedArtifactIdentifier: context.artifactId,
    },
  });
}

function writeReleaseProvenanceAndManifest(context, built, notarized, candidate) {
  const localGateResults = Object.fromEntries(LOCAL_GATE_LABELS.map((label) => {
    const execution = captures.get(label)?.execution;
    if (!execution || execution.exit_code !== 0) {
      throw new Error(`Release provenance is missing passing gate evidence for ${label}.`);
    }
    return [label, execution];
  }));
  const candidateIntegrity = candidate.candidateIntegrity;
  const provenance = {
    schemaVersion: 1,
    kind: "oomu.release-provenance",
    releaseVersion,
    workflowSourceCommit: context.sourceRevision,
    releasePolicyId: immutableReleaseToolchain.policy.policyId,
    releasePolicyDigest: immutableReleaseToolchain.policyDigest,
    runnerIdentity: immutableReleaseToolchain.runner,
    actionCommitShas: immutableReleaseToolchain.policy.actions,
    executableEvidence: immutableReleaseToolchain.tools,
    executableVersions: immutableReleaseToolchain.versions,
    rustToolchain: immutableReleaseToolchain.policy.rust,
    xcodeSdk: immutableReleaseToolchain.runner.xcode,
    unsignedHandoffIdentity: null,
    unsignedTreeDigest: null,
    unsignedPayloadDigest: null,
    unsignedArchiveDigest: null,
    signedOutputDigest: artifactDigestForEntries(collectTreeEntries(context.candidateDir)),
    buildSignPhaseIsolated: false,
    gateResults: localGateResults,
    signing: built.signedDetails,
    notarization: {
      app: notarized.appNotaryResult,
      dmg: notarized.dmgNotaryResult,
    },
    releaseCandidateIntegrity: {
      kind: candidateIntegrity.kind,
      schemaVersion: candidateIntegrity.schemaVersion,
      reportSha256: sha256(readFileSync(candidate.candidateIntegrityPath)),
      applicationTreeDigest: candidateIntegrity.application.treeDigest,
      codeObjectCount: candidateIntegrity.application.codeObjectCount,
      bundleIdentifier: candidateIntegrity.application.bundleIdentifier,
      teamId: candidateIntegrity.application.teamId,
      authority: candidateIntegrity.application.authority,
      buildNumber: candidateIntegrity.application.buildNumber,
      codeDirectoryHash: candidateIntegrity.application.codeDirectoryHash,
      designatedRequirementSha256:
        candidateIntegrity.application.designatedRequirementSha256,
      hardenedRuntime: candidateIntegrity.application.hardenedRuntime,
      entitlementDigest: candidateIntegrity.entitlements.canonicalSha256,
      gatekeeperAccepted:
        candidateIntegrity.application.gatekeeperAccepted
        && candidateIntegrity.container.gatekeeperAccepted,
      notarizationAccepted:
        candidateIntegrity.application.notarizationTicketValidated
        && candidateIntegrity.container.notarizationTicketValidated,
    },
    generatedAt: new Date().toISOString(),
  };
  const releaseProvenancePath = join(context.evidenceDir, "release-provenance.json");
  writeFileSync(
    releaseProvenancePath,
    `${JSON.stringify(provenance, null, 2)}\n`,
    { mode: 0o400, flag: "wx" },
  );
  makeTreeImmutable(context.candidateDir);
  const manifestPath = join(context.evidenceDir, "MANIFEST.json");
  runStep("manifest_generation", context.node, [
    "scripts/release-manifest.mjs", "generate",
    "--tree", context.candidateDir,
    "--manifest", manifestPath,
    "--build-id", context.buildId,
    "--source-revision", context.sourceRevision,
    "--artifact-id", context.artifactId,
    "--private-key", context.privateKeyPath,
    "--provenance", releaseProvenancePath,
  ], {
    evidenceArgs: [
      "scripts/release-manifest.mjs", "generate",
      "--tree", context.candidateDir,
      "--manifest", manifestPath,
      "--build-id", context.buildId,
      "--source-revision", context.sourceRevision,
      "--artifact-id", context.artifactId,
      "--private-key", "<secure-key-path>",
      "--provenance", releaseProvenancePath,
    ],
  });
  runStep("manifest_verification", context.node, [
    "scripts/release-manifest.mjs", "verify",
    "--tree", context.candidateDir,
    "--manifest", manifestPath,
    "--build-id", context.buildId,
    "--source-revision", context.sourceRevision,
    "--artifact-id", context.artifactId,
    "--public-key", context.publicKeyPath,
    "--provenance", releaseProvenancePath,
  ]);
  const manifest = readJson(manifestPath, "Release manifest");
  return {
    releaseProvenancePath,
    manifestPath,
    manifest,
    artifactDigest: manifest.artifact_digest,
    dmgSha256: sha256(readFileSync(candidate.stagedDmg)),
  };
}

function qualifyAndMaterializeRelease(
  context,
  gates,
  built,
  notarized,
  candidate,
  releaseEvidence,
  candidateDescriptor,
) {
  const qualification = runCleanMachineQualification({
    ...context,
    ...candidate,
    ...releaseEvidence,
    codesign: built.codesign,
    mainBinaryName: tauriConfig.mainBinaryName,
    sha256,
    readJson,
    runStep,
    externalHarnessEnvironment,
    makeTreeImmutable,
    expectedBuildNumber: releaseVersion.buildNumber,
    expectedBundleIdentifier: tauriConfig.identifier,
  });
  const canonicalEvidence = materializeCanonicalReleaseEvidence({
    ...context,
    ...gates,
    ...built,
    ...notarized,
    ...candidate,
    ...releaseEvidence,
    ...qualification,
    signedCandidateDescriptor: candidateDescriptor,
    repositoryRoot: root,
  });
  console.log(
    `[release] COMPLETE artifact=${context.artifactId} digest=${releaseEvidence.artifactDigest}`,
  );
  console.log(`[release] Candidate: ${context.candidateDir}`);
  console.log(`[release] Evidence: ${context.evidenceDir}`);
  console.log(`OOMU_RELEASE_CANDIDATE_MANIFEST=${releaseEvidence.manifestPath}`);
  console.log(
    `OOMU_RELEASE_CANDIDATE_EVIDENCE=${canonicalEvidence.candidateIntegrityRecordPath}`,
  );
}

function main() {
  const context = initializeReleaseContext();
  const gates = runAutomatedReleaseGates(context);
  const built = buildAndSignApplication(context, immutableReleaseToolchain);
  const notarized = notarizeAndCreateDmg(context, immutableReleaseToolchain, built);
  const candidate = stageAndVerifyCandidate(
    context, immutableReleaseToolchain, built, notarized,
  );
  const releaseEvidence = writeReleaseProvenanceAndManifest(
    context, built, notarized, candidate,
  );
  const candidateDescriptor = materializeCandidateDescriptor(
    context, candidate, releaseEvidence,
  );
  console.log(`OOMU_SIGNED_CANDIDATE_DESCRIPTOR=${candidateDescriptor.path}`);

  qualifyAndMaterializeRelease(
    context, gates, built, notarized, candidate, releaseEvidence, candidateDescriptor,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU RELEASE FAILED: ${error.message}`);
    process.exit(1);
  }
}
