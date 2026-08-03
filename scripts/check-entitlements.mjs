#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { atomicWriteFile } from "./release-manifest.mjs";
import { verifyReviewedSourcePermissionContinuity } from "./release-gates/macos-permission-continuity.mjs";

const repoRoot = resolve(import.meta.dirname, "..");
const defaultEntitlements = resolve(repoRoot, "src-tauri", "entitlements.plist");
const defaultSnapshot = resolve(repoRoot, "release", "entitlements.snapshot.json");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function parseEntitlementsPlist(contents) {
  const result = spawnSync(
    "/usr/bin/plutil",
    ["-convert", "json", "-o", "-", "--", "-"],
    { input: contents, encoding: "utf8" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Unable to parse entitlement plist: ${result.stderr.trim()}`);
  }
  const values = JSON.parse(result.stdout);
  if (!values || Array.isArray(values) || typeof values !== "object") {
    throw new Error("Entitlement plist root must be a dictionary.");
  }
  if (Object.keys(values).length === 0) {
    throw new Error("No entitlements were found in the plist.");
  }
  return values;
}

function canonicalValue(value) {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonicalValue(child)]),
    );
  }
  return value;
}

export function canonicalEntitlements(values) {
  return JSON.stringify(canonicalValue(values));
}

export function verifySignedEntitlements({ artifactPath, expectedEntitlements, label }) {
  const artifact = resolve(artifactPath);
  const result = spawnSync(
    "/usr/bin/codesign",
    ["-d", "--entitlements", ":-", artifact],
    { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
  );
  if (result.error) throw result.error;
  if (result.status !== 0 || !result.stdout.trim()) {
    throw new Error(`${label} has no extractable signed entitlements.`);
  }
  const entitlements = parseEntitlementsPlist(result.stdout);
  const actualCanonical = canonicalEntitlements(entitlements);
  const expectedCanonical = canonicalEntitlements(expectedEntitlements);
  if (actualCanonical !== expectedCanonical) {
    throw new Error(`${label} signed entitlements differ from the reviewed exact set.`);
  }
  return {
    artifact,
    entitlements,
    canonical_sha256: sha256(actualCanonical),
    extraction: {
      executable: "/usr/bin/codesign",
      arguments: ["-d", "--entitlements", ":-", basename(artifact)],
      exit_code: 0,
    },
  };
}

export function verifyEntitlementSnapshot({
  entitlementsPath = defaultEntitlements,
  snapshotPath = defaultSnapshot,
} = {}) {
  const actual = parseEntitlementsPlist(readFileSync(resolve(entitlementsPath), "utf8"));
  const snapshot = JSON.parse(readFileSync(resolve(snapshotPath), "utf8"));
  if (snapshot.schema_version !== 1 || snapshot.kind !== "oomu.reviewed-entitlements") {
    throw new Error("Unsupported entitlement snapshot schema.");
  }
  const actualCanonical = canonicalEntitlements(actual);
  const expectedCanonical = canonicalEntitlements(snapshot.reviewed_entitlements ?? {});
  if (actualCanonical !== expectedCanonical) {
    const added = Object.keys(actual).filter(
      (key) => !Object.hasOwn(snapshot.reviewed_entitlements ?? {}, key),
    );
    const removed = Object.keys(snapshot.reviewed_entitlements ?? {}).filter(
      (key) => !Object.hasOwn(actual, key),
    );
    throw new Error(
      `Entitlements differ from the reviewed least-privilege snapshot (added: ${added.join(", ") || "none"}; removed: ${removed.join(", ") || "none"}).`,
    );
  }
  const actualHash = sha256(actualCanonical);
  if (snapshot.sha256 !== actualHash) {
    throw new Error("Entitlement snapshot digest is invalid.");
  }
  return { path: resolve(entitlementsPath), entitlements: actual, sha256: actualHash };
}

export function verifyFinalSignedEntitlements({ appPath }) {
  const applicationPolicy = verifyEntitlementSnapshot();
  const application = verifySignedEntitlements({
    artifactPath: appPath,
    expectedEntitlements: applicationPolicy.entitlements,
    label: "Application",
  });
  return {
    schema_version: 1,
    kind: "oomu.final-signed-entitlement-snapshot",
    status: "passed",
    synthetic: false,
    completed_at: new Date().toISOString(),
    application: {
      reviewed_policy: {
        path: "src-tauri/entitlements.plist",
        canonical_sha256: applicationPolicy.sha256,
      },
      extracted: application,
    },
  };
}

function parseCliArguments(argv) {
  if (argv.length === 0) return null;
  const values = {};
  const allowed = new Set(["--signed-app", "--output"]);
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!allowed.has(key) || !value || value.startsWith("--")) {
      throw new Error(
        "Usage: check-entitlements.mjs --signed-app <app> --output <json>",
      );
    }
    if (values[key] !== undefined) throw new Error(`Duplicate argument: ${key}`);
    values[key] = resolve(value);
  }
  for (const key of allowed) {
    if (!values[key]) throw new Error(`Required argument missing: ${key}`);
  }
  return values;
}

function main() {
  verifyReviewedSourcePermissionContinuity();
  const args = parseCliArguments(process.argv.slice(2));
  if (args) {
    const report = verifyFinalSignedEntitlements({
      appPath: args["--signed-app"],
    });
    atomicWriteFile(args["--output"], `${JSON.stringify(report, null, 2)}\n`, 0o600);
    console.log("Final signed app entitlement snapshot passed.");
    return;
  }
  const result = verifyEntitlementSnapshot();
  console.log(`Entitlement snapshot passed: app=${result.sha256}.`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU ENTITLEMENT ERROR: ${error.message}`);
    process.exit(1);
  }
}
