#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const name = "macos_permission_continuity";

const repoRoot = resolve(import.meta.dirname, "../..");
const reviewedSnapshotPath = resolve(
  repoRoot,
  "release/macos-permission-continuity.snapshot.json",
);

function execute(command, args, input) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    input,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${basename(command)} failed: ${(result.stderr || result.stdout).trim()}`);
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

function plist(path) {
  return JSON.parse(execute("/usr/bin/plutil", ["-convert", "json", "-o", "-", "--", path]));
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonical(child)]),
    );
  }
  return value;
}

function digest(value) {
  return createHash("sha256").update(JSON.stringify(canonical(value))).digest("hex");
}

function usageKeys(info) {
  return Object.keys(info)
    .filter((key) => /^NS.+UsageDescription$/u.test(key))
    .sort();
}

function sourceHelpers(config) {
  return (config.bundle?.externalBin ?? [])
    .map((path) => basename(path))
    .sort();
}

export function sourcePermissionSnapshot({ root = repoRoot } = {}) {
  const config = JSON.parse(readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8"));
  const info = plist(resolve(root, "src-tauri/Info.plist"));
  const entitlements = plist(resolve(root, "src-tauri/entitlements.plist"));
  const snapshot = {
    schema_version: 1,
    kind: "oomu.macos-permission-continuity",
    bundle_identifier: config.identifier,
    main_executable: config.mainBinaryName,
    usage_description_keys: usageKeys(info),
    entitlements: canonical(entitlements),
    helper_executables: sourceHelpers(config),
  };
  return { ...snapshot, canonical_sha256: digest(snapshot) };
}

function signature(path) {
  const detail = execute("/usr/bin/codesign", ["-d", "--verbose=4", "-r-", path]);
  const designatedRequirement = detail.match(/^designated => (.+)$/mu)?.[1] ?? null;
  return {
    identifier: detail.match(/^Identifier=(.+)$/mu)?.[1] ?? null,
    team_identifier: detail.match(/^TeamIdentifier=(.+)$/mu)?.[1] ?? null,
    authority_chain: [...detail.matchAll(/^Authority=(.+)$/gmu)].map((match) => match[1]),
    designated_requirement: designatedRequirement,
    designated_requirement_sha256: designatedRequirement
      ? createHash("sha256").update(designatedRequirement).digest("hex")
      : null,
    hardened_runtime: /\bflags=0x[0-9a-f]+\(runtime\)/iu.test(detail),
  };
}

function bundleBuildIdentity(info) {
  if (Number.isSafeInteger(info.OOMUBuildNumber) && info.OOMUBuildNumber > 0) {
    return { build_number: info.OOMUBuildNumber, build_number_source: "bundle" };
  }
  const lineage = JSON.parse(
    readFileSync(resolve(repoRoot, "release/macos-permission-lineage.json"), "utf8"),
  );
  if (
    info.CFBundleIdentifier === lineage.bundleIdentifier
    && info.CFBundleShortVersionString === lineage.firstProductVersion
    && info.CFBundleVersion === lineage.firstProductVersion
  ) {
    return {
      build_number: lineage.firstBuildNumber,
      build_number_source: "reviewed_first_release_legacy",
    };
  }
  throw new Error("The signed app has no trustworthy release build number.");
}

function signedEntitlements(path) {
  const result = spawnSync(
    "/usr/bin/codesign",
    ["-d", "--entitlements", ":-", path],
    { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 },
  );
  if (result.error) throw result.error;
  if (result.status !== 0 || !result.stdout.trim()) {
    throw new Error(`codesign could not read signed entitlements for ${basename(path)}.`);
  }
  return JSON.parse(
    execute(
      "/usr/bin/plutil",
      ["-convert", "json", "-o", "-", "--", "-"],
      result.stdout,
    ),
  );
}

export function signedPermissionSnapshot(appPath) {
  const app = resolve(appPath);
  execute("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=4", app]);
  const info = plist(resolve(app, "Contents/Info.plist"));
  const buildIdentity = bundleBuildIdentity(info);
  const macos = resolve(app, "Contents/MacOS");
  const mainExecutable = info.CFBundleExecutable;
  const helpers = readdirSync(macos)
    .filter((name) => name !== mainExecutable)
    .sort()
    .map((name) => ({ name, ...signature(resolve(macos, name)) }));
  const entitlements = signedEntitlements(resolve(macos, mainExecutable));
  const snapshot = {
    schema_version: 2,
    kind: "oomu.macos-permission-continuity.signed",
    bundle_identifier: info.CFBundleIdentifier,
    product_version: info.CFBundleShortVersionString,
    bundle_version: info.CFBundleVersion,
    ...buildIdentity,
    main_executable: mainExecutable,
    usage_description_keys: usageKeys(info),
    entitlements: canonical(entitlements),
    application_signature: signature(resolve(macos, mainExecutable)),
    helpers,
    strict_code_signature: "valid",
  };
  return { ...snapshot, canonical_sha256: digest(snapshot) };
}

const SIGNATURE_KEYS = [
  "identifier",
  "team_identifier",
  "authority_chain",
  "designated_requirement",
  "designated_requirement_sha256",
  "hardened_runtime",
];

function unequal(left, right) {
  return JSON.stringify(canonical(left)) !== JSON.stringify(canonical(right));
}

function identityFailures(previous, current) {
  return ["bundle_identifier", "main_executable", "entitlements"]
    .filter((key) => unequal(previous[key], current[key]));
}

function usageFailures(previous, current, approvedUsage) {
  const prior = previous.usage_description_keys ?? [];
  const next = current.usage_description_keys ?? [];
  const removed = prior.some((key) => !next.includes(key));
  const unapproved = next.some((key) => !prior.includes(key) && !approvedUsage.includes(key));
  return removed || unapproved ? ["usage_description_keys"] : [];
}

function helperInventory(snapshot) {
  return snapshot.helper_executables
    ?? snapshot.helpers?.map((helper) => helper.name).sort()
    ?? [];
}

function signatureFailures(prefix, previous, current) {
  if (!previous) return [];
  return SIGNATURE_KEYS
    .filter((key) => unequal(previous[key], current?.[key]))
    .map((key) => `${prefix}.${key}`);
}

function helperSignatureFailures(previous, current) {
  if (!previous.helpers || !current.helpers) return [];
  const prior = new Map(previous.helpers.map((helper) => [helper.name, helper]));
  return current.helpers.flatMap((helper) => {
    const old = prior.get(helper.name);
    return old ? signatureFailures(`helper.${helper.name}`, old, helper) : [];
  });
}

export function comparePermissionContinuity(previous, current, options = {}) {
  const failures = [
    ...identityFailures(previous, current),
    ...usageFailures(previous, current, options.approvedUsageKeyAdditions ?? []),
    ...(unequal(helperInventory(previous), helperInventory(current))
      ? ["helper_executables"]
      : []),
    ...signatureFailures(
      "application_signature",
      previous.application_signature,
      current.application_signature,
    ),
    ...helperSignatureFailures(previous, current),
  ];
  if (
    current.schema_version === 2
    && current.bundle_version !== String(current.build_number)
  ) {
    failures.push("bundle_version");
  }
  if (
    options.requireBuildIncrease
    && (!Number.isSafeInteger(previous.build_number)
      || !Number.isSafeInteger(current.build_number)
      || current.build_number <= previous.build_number)
  ) {
    failures.push("build_number");
  }
  if (current.strict_code_signature && current.strict_code_signature !== "valid") {
    failures.push("strict_code_signature");
  }
  if (failures.length > 0) {
    throw new Error(
      `macOS permission identity changed without a reviewed continuity update: ${[...new Set(failures)].join(", ")}`,
    );
  }
  return true;
}

export function verifyReviewedSourcePermissionContinuity({ root = repoRoot } = {}) {
  const reviewed = JSON.parse(readFileSync(reviewedSnapshotPath, "utf8"));
  const current = sourcePermissionSnapshot({ root });
  comparePermissionContinuity(reviewed, current);
  return current;
}

function requireDeveloperIdIdentity(snapshot) {
  const application = snapshot.application_signature;
  const teamIdentifier = application?.team_identifier;
  const developerIdAuthority = application?.authority_chain?.some((authority) =>
    authority.startsWith("Developer ID Application:"),
  );
  if (
    snapshot.strict_code_signature !== "valid"
    || application?.identifier !== snapshot.bundle_identifier
    || !/^[A-Z0-9]{10}$/u.test(teamIdentifier ?? "")
    || !developerIdAuthority
    || application?.hardened_runtime !== true
  ) {
    throw new Error("The packaged application does not have a valid Developer ID identity.");
  }
  const invalidHelper = snapshot.helpers?.find((helper) =>
    helper.team_identifier !== teamIdentifier
    || helper.hardened_runtime !== true
    || !helper.authority_chain?.some((authority) =>
      authority.startsWith("Developer ID Application:"),
    ),
  );
  if (invalidHelper) {
    throw new Error(`The packaged helper ${invalidHelper.name} does not match the application identity.`);
  }
  return teamIdentifier;
}

export async function run({ appPath, root = repoRoot }) {
  const source = verifyReviewedSourcePermissionContinuity({ root });
  const signed = signedPermissionSnapshot(appPath);
  comparePermissionContinuity(source, signed);
  const teamIdentifier = requireDeveloperIdIdentity(signed);
  return {
    passed: true,
    evidence: {
      schema_version: 1,
      recursive: true,
      source_identity_sha256: source.canonical_sha256,
      signed_identity_sha256: signed.canonical_sha256,
      bundle_identifier: signed.bundle_identifier,
      product_version: signed.product_version,
      build_number: signed.build_number,
      team_identifier: teamIdentifier,
      helper_count: signed.helpers.length,
      strict_code_signature: signed.strict_code_signature,
    },
  };
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function hasArgument(name) {
  return process.argv.includes(name);
}

function main() {
  const snapshotApp = argumentValue("--snapshot-app");
  const outputPath = argumentValue("--output");
  const expectedTeam = argumentValue("--expected-team");
  if (snapshotApp) {
    if (!outputPath) throw new Error("Use --snapshot-app <app> --output <snapshot.json>.");
    const snapshot = signedPermissionSnapshot(snapshotApp);
    verifyExpectedTeam(snapshot, expectedTeam);
    if (hasArgument("--first-signed-release")) {
      snapshot.continuity_review = {
        first_signed_release: true,
        previous_snapshot: null,
      };
    }
    writeFileSync(resolve(outputPath), `${JSON.stringify(snapshot, null, 2)}\n`, { mode: 0o600 });
    console.log("macOS signed permission identity snapshot recorded.");
    return;
  }
  const signedApp = argumentValue("--signed-app");
  if (!signedApp) {
    verifyReviewedSourcePermissionContinuity();
    console.log("macOS permission identity source gate passed.");
    return;
  }
  const previousPath = argumentValue("--previous");
  if (!previousPath || !outputPath) {
    throw new Error("Use --signed-app <app> --previous <snapshot.json> --output <snapshot.json>.");
  }
  const previous = JSON.parse(readFileSync(resolve(previousPath), "utf8"));
  const current = signedPermissionSnapshot(signedApp);
  verifyExpectedTeam(current, expectedTeam);
  const reviewed = JSON.parse(readFileSync(reviewedSnapshotPath, "utf8"));
  comparePermissionContinuity(previous, current, {
    approvedUsageKeyAdditions: reviewed.approved_n_plus_one_usage_key_additions ?? [],
    requireBuildIncrease: true,
  });
  current.continuity_review = {
    previous_snapshot: resolve(previousPath),
    approved_usage_key_additions:
      reviewed.approved_n_plus_one_usage_key_additions ?? [],
  };
  writeFileSync(resolve(outputPath), `${JSON.stringify(current, null, 2)}\n`, { mode: 0o600 });
  console.log("macOS signed permission continuity gate passed.");
}

function verifyExpectedTeam(snapshot, expectedTeam) {
  if (!expectedTeam) return;
  if (snapshot.application_signature?.team_identifier !== expectedTeam) {
    throw new Error("The signed app Team ID does not match the reviewed release team.");
  }
  const wrongHelper = snapshot.helpers?.find(
    (helper) => helper.team_identifier !== expectedTeam,
  );
  if (wrongHelper) {
    throw new Error(`The signed helper ${wrongHelper.name} does not match the reviewed release team.`);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`MACOS PERMISSION CONTINUITY GATE FAILED: ${error.message}`);
    process.exit(1);
  }
}
