#!/usr/bin/env node

import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import process from "node:process";
import { createUnsignedHandoff } from "./release-handoff.mjs";
import { canonicalRustPathRemapEnvironment } from "./release-environment.mjs";
import {
  collectReleaseToolchain,
  runApproved,
  sha256Bytes,
} from "./release-provenance.mjs";

const root = resolve(import.meta.dirname, "..");
const NPM_GATES = [
  ["lint", ["run", "lint"]],
  ["version", ["run", "check:version"]],
  ["source-size", ["run", "check:source-size"]],
  ["real-components", ["run", "check:real-components"]],
  ["p0-architecture", ["run", "check:p0-architecture"]],
  ["p1-contracts", ["run", "check:p1-contracts"]],
  ["novice-ui", ["run", "check:novice-ui"]],
  ["module-cycles", ["run", "check:module-cycles"]],
  ["unused-exports", ["run", "check:unused-exports"]],
  ["repository-hygiene", ["run", "check:repository-hygiene"]],
  ["release-integrity", ["run", "test:release-integrity"]],
  ["i18n", ["run", "check:i18n"]],
  ["typecheck", ["run", "typecheck"]],
  ["frontend", ["run", "test:frontend"]],
];

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required for the unsigned release phase.`);
  return value;
}

function exactSourceRevision(toolchain) {
  const revision = runApproved(toolchain, "git", ["rev-parse", "HEAD"], {
    label: "resolve unsigned source revision",
  }).stdout.trim();
  if (!/^[0-9a-f]{40}$/iu.test(revision)) throw new Error("Source revision is not immutable.");
  const dirty = runApproved(
    toolchain,
    "git",
    ["status", "--porcelain=v1", "--untracked-files=all"],
    { label: "verify unsigned source tree" },
  ).stdout.trim();
  if (dirty) throw new Error("Unsigned release source tree must be clean.");
  return revision;
}

function captureStep(toolchain, tool, label, args, environment, validate = null) {
  const result = runApproved(toolchain, tool, args, {
    label,
    environment,
    exposeFailureOutput: true,
  });
  if (validate) validate(result.stdout ?? "");
  return {
    label,
    executable: toolchain.tools[tool].executable,
    executableSha256: toolchain.tools[tool].sha256,
    exitCode: result.status,
    stdoutSha256: sha256Bytes(Buffer.from(result.stdout ?? "", "utf8")),
    stderrSha256: sha256Bytes(Buffer.from(result.stderr ?? "", "utf8")),
  };
}

function findNamed(directory, predicate) {
  const matches = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory() && !predicate(entry)) walk(path);
      if (predicate(entry)) matches.push(path);
    }
  };
  walk(directory);
  return matches;
}

function runSourceQualification(toolchain, environment) {
  const results = NPM_GATES.map(([label, args]) =>
    captureStep(toolchain, "npm", label, args, environment));
  const nodeSteps = [
    ["rust-dependency-audit", ["scripts/audit-rust-dependencies.mjs"]],
    ["entitlement-snapshot", ["scripts/check-entitlements.mjs"]],
  ];
  results.push(...nodeSteps.map(([label, args]) =>
    captureStep(toolchain, "node", label, args, environment)));
  results.push(captureStep(
    toolchain,
    "npm",
    "dependency-audit",
    ["audit", "--omit=dev", "--audit-level=high", "--json"],
    environment,
    (output) => JSON.parse(output),
  ));
  results.push(captureStep(
    toolchain,
    "node",
    "external-bin-manifest-reservation",
    ["scripts/prepare-tauri-external-bins.mjs"],
    environment,
  ));
  results.push(captureStep(
    toolchain,
    "node",
    "portable-python-preparation",
    ["scripts/prepare-portable-python.mjs", "--release"],
    environment,
  ));
  const cargoSteps = [
    ["pdf-containment", [
      "test", "--locked", "--target", toolchain.policy.target,
      "--manifest-path", "src-tauri/Cargo.toml", "--test", "pdf_containment",
      "real_corpus_processes_are_bounded_and_deterministic", "--", "--exact", "--nocapture",
    ]],
    ["cargo-check", [
      "check", "--locked", "--target", toolchain.policy.target,
      "--manifest-path", "src-tauri/Cargo.toml",
    ]],
    ["cargo-test", [
      "test", "--locked", "--target", toolchain.policy.target,
      "--manifest-path", "src-tauri/Cargo.toml",
      "--", "--test-threads=1",
    ]],
  ];
  results.push(...cargoSteps.map(([label, args]) =>
    captureStep(toolchain, "cargo", label, args, environment)));
  return results;
}

function buildUnsignedApplication(toolchain, environment, results) {
  const bundleRoot = join(
    root, "src-tauri", "target", toolchain.policy.target, "release", "bundle",
  );
  rmSync(bundleRoot, { recursive: true, force: true });
  results.push(captureStep(
    toolchain,
    "npm",
    "native-release-preparation",
    ["run", "tauri:prepare"],
    environment,
  ));
  const buildSteps = [
    ["compile", [
      "run", "tauri:build:internal", "--", "--target", toolchain.policy.target,
      "--config", "src-tauri/tauri.release.conf.json", "--no-bundle",
    ]],
    ["unsigned-bundle", [
      "run", "tauri:bundle:internal", "--", "--target", toolchain.policy.target,
      "--config", "src-tauri/tauri.release.conf.json", "--bundles", "app", "--no-sign",
    ]],
  ];
  results.push(...buildSteps.map(([label, args]) =>
    captureStep(toolchain, "npm", label, args, environment)));
  const apps = findNamed(bundleRoot, (entry) =>
    entry.isDirectory() && entry.name.endsWith(".app"));
  if (apps.length !== 1) throw new Error("Unsigned build must produce exactly one app bundle.");
  return realpathSync(apps[0]);
}

function sanitizeUnsignedApplication({
  appPath,
  buildIdentifier,
  environment,
  evidenceDirectory,
  results,
  toolchain,
}) {
  const sanitizers = [
    ["unsigned-sanitizer", "scripts/sanitize-release.mjs", "release-sanitizer.json", [
      "--dir", appPath, "--execute",
    ]],
    ["unsigned-database-sanitizer", "scripts/sanitize-release-db.mjs", "database-sanitizer.json", [
      "--release", "--dir", appPath,
    ]],
  ];
  for (const [label, script, evidenceName, args] of sanitizers) {
    const evidencePath = join(evidenceDirectory, evidenceName);
    const result = captureStep(toolchain, "node", label, [
      script, ...args, "--build-id", buildIdentifier, "--evidence", evidencePath,
    ], environment);
    result.evidenceSha256 = sha256Bytes(readFileSync(evidencePath));
    results.push(result);
  }
  const assetCatalogs = findNamed(appPath, (entry) =>
    entry.isFile() && entry.name === "Assets.car");
  if (assetCatalogs.length !== 1) {
    throw new Error("Unsigned application must contain exactly one compiled asset catalog.");
  }
  results.push(captureStep(
    toolchain,
    "assetutil",
    "asset-catalog-validation",
    ["--info", assetCatalogs[0]],
    environment,
    (output) => {
      const catalog = JSON.parse(output);
      if (!Array.isArray(catalog) || catalog.length === 0) {
        throw new Error("Unsigned application asset catalog is empty.");
      }
    },
  ));
}

function main() {
  if (process.env.GITHUB_ACTIONS !== "true") {
    throw new Error("The isolated unsigned phase is reserved for the protected CI workflow.");
  }
  const toolchain = collectReleaseToolchain({ ciPhase: true, protectedPhase: false });
  const sourceRevision = exactSourceRevision(toolchain);
  if (sourceRevision.toLowerCase() !== required("OOMU_SOURCE_REVISION").toLowerCase()) {
    throw new Error("Checked-out source differs from the workflow-authorized revision.");
  }
  const buildIdentifier = required("OOMU_BUILD_ID");
  const outputDirectory = resolve(required("OOMU_UNSIGNED_HANDOFF_DIR"));
  if (existsSync(outputDirectory)) throw new Error("Unsigned handoff output must be new.");
  const evidenceDirectory = mkdtempSync(join(tmpdir(), "oomu-unsigned-evidence-"));
  const environment = {
    OOMU_RELEASE_PIPELINE: "unsigned-v2",
    OOMU_BUILD_ID: buildIdentifier,
    OOMU_SOURCE_REVISION: sourceRevision,
    OOMU_RELEASE_POLICY_SHA256: toolchain.policyDigest,
    ...canonicalRustPathRemapEnvironment(root),
  };
  const gateResults = runSourceQualification(toolchain, environment);
  const appPath = buildUnsignedApplication(toolchain, environment, gateResults);
  sanitizeUnsignedApplication({
    appPath,
    buildIdentifier,
    environment,
    evidenceDirectory,
    results: gateResults,
    toolchain,
  });
  exactSourceRevision(toolchain);
  const descriptor = createUnsignedHandoff({
    repositoryRoot: root,
    appPath,
    outputDirectory,
    buildIdentifier,
    sourceRevision,
    toolchain,
    gateResults,
  });
  process.stdout.write(`OOMU_UNSIGNED_HANDOFF_SHA256=${descriptor.handoffSha256}\n`);
  process.stdout.write(`OOMU_UNSIGNED_ARCHIVE=${basename(descriptor.archivePath)}\n`);
  process.stdout.write(
    `OOMU_UNSIGNED_ARTIFACT_IDENTITY=${descriptor.handoff.artifactIdentity}\n`,
  );
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU UNSIGNED RELEASE FAILED: ${error.message}`);
    process.exit(1);
  }
}
