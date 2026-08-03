#!/usr/bin/env node

import { readFileSync, realpathSync } from "node:fs";
import { join, resolve } from "node:path";
import process from "node:process";

const DEFAULT_ROOT = resolve(import.meta.dirname, "..");
const PUBLIC_BETA_VERSION = "0.1.0";
const PUBLIC_BETA_LABEL = "OOMU 0.1 — Public Beta";
const PUBLIC_BETA_TAG = "v0.1.0";
const PLAIN_SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;
const NIGHTLY_SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.0-nightly\.(\d{8})\.([1-9]\d*)$/u;
const RC_SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.0-rc\.([1-9]\d*)$/u;

function fail(message) {
  throw new Error(`Release version contract: ${message}`);
}

function parseJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
}

function assertExactKeys(record) {
  const expected = [
    "buildNumber",
    "channel",
    "intendedTag",
    "productVersion",
    "publicLabel",
    "schemaVersion",
  ];
  const actual = Object.keys(record).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail(`version.json keys must be exactly ${expected.join(", ")}.`);
  }
}

function assertRealDate(value) {
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(4, 6));
  const day = Number(value.slice(6, 8));
  const date = new Date(Date.UTC(year, month - 1, day));
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    fail(`nightly date ${value} is not a real calendar date.`);
  }
}

export function assertChannelVersion(productVersion, channel) {
  if (channel === "beta") {
    const match = productVersion.match(PLAIN_SEMVER);
    if (!match || Number(match[1]) !== 0) {
      fail("beta versions must be plain 0.x.y semantic versions.");
    }
    return;
  }
  if (channel === "nightly") {
    const match = productVersion.match(NIGHTLY_SEMVER);
    if (!match || Number(match[1]) !== 0 || Number(match[2]) < 1) {
      fail("nightly versions must match 0.x.0-nightly.YYYYMMDD.N.");
    }
    assertRealDate(match[3]);
    return;
  }
  if (channel === "rc") {
    const match = productVersion.match(RC_SEMVER);
    if (!match || Number(match[1]) !== 0 || Number(match[2]) < 1) {
      fail("release candidates must match 0.x.0-rc.N.");
    }
    return;
  }
  if (channel === "stable") {
    const match = productVersion.match(PLAIN_SEMVER);
    if (!match || Number(match[1]) < 1) {
      fail("stable versions must be plain semantic versions at or above 1.0.0.");
    }
    return;
  }
  fail(`unsupported channel ${JSON.stringify(channel)}.`);
}

export function assertMonotonicBuildNumber(previous, next) {
  if (
    !Number.isSafeInteger(previous) ||
    previous < 0 ||
    !Number.isSafeInteger(next) ||
    next < 1 ||
    next <= previous
  ) {
    fail("a signed candidate build number must be a positive integer greater than its predecessor.");
  }
}

export function validateReleaseVersionRecord(record) {
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    fail("version.json must contain one object.");
  }
  assertExactKeys(record);
  if (record.schemaVersion !== 1) fail("schemaVersion must be 1.");
  assertChannelVersion(record.productVersion, record.channel);
  if (!Number.isSafeInteger(record.buildNumber) || record.buildNumber < 1) {
    fail("buildNumber must be a positive integer.");
  }
  if (record.intendedTag !== `v${record.productVersion}`) {
    fail("intendedTag must be v followed by productVersion.");
  }
  if (record.productVersion === PUBLIC_BETA_VERSION) {
    if (record.channel !== "beta") fail("0.1.0 must use the beta channel.");
    if (record.publicLabel !== PUBLIC_BETA_LABEL) {
      fail(`0.1.0 publicLabel must be exactly ${PUBLIC_BETA_LABEL}.`);
    }
    if (record.intendedTag !== PUBLIC_BETA_TAG) {
      fail(`0.1.0 intendedTag must be exactly ${PUBLIC_BETA_TAG}.`);
    }
  } else if (typeof record.publicLabel !== "string" || record.publicLabel.trim() === "") {
    fail("publicLabel must be a non-empty string.");
  }
  return Object.freeze({ ...record });
}

export function loadReleaseVersionRecord(repositoryRoot = DEFAULT_ROOT) {
  const record = parseJson(
    join(repositoryRoot, "release", "version.json"),
    "release/version.json",
  );
  return validateReleaseVersionRecord(record);
}

export function releaseArtifactIdentifier(record, buildIdentifier) {
  const checked = validateReleaseVersionRecord(record);
  if (!/^[A-Za-z0-9._-]{8,128}$/u.test(buildIdentifier ?? "")) {
    fail("build identifier is invalid.");
  }
  return `oomu-macos-${checked.productVersion}-build.${checked.buildNumber}-${buildIdentifier}`;
}

export function releaseDmgName(record, productName) {
  const checked = validateReleaseVersionRecord(record);
  if (!/^[A-Za-z0-9._-]+$/u.test(productName ?? "")) fail("product name is invalid.");
  return `${productName}-${checked.productVersion}.dmg`;
}

export function unsignedReleaseArtifactIdentity(record, buildIdentifier) {
  return `${releaseArtifactIdentifier(record, buildIdentifier)}-unsigned`;
}

export function unsignedReleaseArchiveName(record, productName) {
  const checked = validateReleaseVersionRecord(record);
  if (!/^[A-Za-z0-9._-]+$/u.test(productName ?? "")) fail("product name is invalid.");
  return `${productName}-${checked.productVersion}-build.${checked.buildNumber}-unsigned.zip`;
}

function readCargoPackageVersion(path, packageName) {
  const source = readFileSync(path, "utf8");
  const packageBlock = source.match(/\[package\]([\s\S]*?)(?=\n\[|$)/u)?.[1] ?? "";
  const name = packageBlock.match(/^\s*name\s*=\s*"([^"]+)"/mu)?.[1];
  const version = packageBlock.match(/^\s*version\s*=\s*"([^"]+)"/mu)?.[1];
  if (name !== packageName || !version) fail(`${path} has no ${packageName} package version.`);
  return version;
}

function readCargoLockPackageVersion(path, packageName) {
  const source = readFileSync(path, "utf8");
  for (const block of source.split("[[package]]").slice(1)) {
    const name = block.match(/^\s*name\s*=\s*"([^"]+)"/mu)?.[1];
    if (name === packageName) {
      const version = block.match(/^\s*version\s*=\s*"([^"]+)"/mu)?.[1];
      if (version) return version;
    }
  }
  fail(`${path} has no ${packageName} package version.`);
}

function readPlistInteger(path, key) {
  const source = readFileSync(path, "utf8");
  const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const match = source.match(
    new RegExp(`<key>${escapedKey}</key>\\s*<integer>([1-9]\\d*)</integer>`, "u"),
  );
  if (!match) fail(`${path} has no positive integer ${key}.`);
  return Number(match[1]);
}

export function checkActiveVersionSurfaces(repositoryRoot = DEFAULT_ROOT) {
  const record = loadReleaseVersionRecord(repositoryRoot);
  const packageJson = parseJson(join(repositoryRoot, "package.json"), "package.json");
  const packageLock = parseJson(join(repositoryRoot, "package-lock.json"), "package-lock.json");
  const tauri = parseJson(
    join(repositoryRoot, "src-tauri", "tauri.conf.json"),
    "src-tauri/tauri.conf.json",
  );
  const actual = {
    "package.json": packageJson.version,
    "package-lock.json": packageLock.version,
    "package-lock.json root package": packageLock.packages?.[""]?.version,
    "src-tauri/Cargo.toml": readCargoPackageVersion(
      join(repositoryRoot, "src-tauri", "Cargo.toml"),
      "oomu",
    ),
    "src-tauri/Cargo.lock": readCargoLockPackageVersion(
      join(repositoryRoot, "src-tauri", "Cargo.lock"),
      "oomu",
    ),
    "src-tauri/tauri.conf.json": tauri.version,
  };
  for (const [surface, version] of Object.entries(actual)) {
    if (version !== record.productVersion) {
      fail(`${surface} reports ${JSON.stringify(version)}; expected ${record.productVersion}.`);
    }
  }
  const bundleBuildNumber = readPlistInteger(
    join(repositoryRoot, "src-tauri", "Info.plist"),
    "OOMUBuildNumber",
  );
  if (bundleBuildNumber !== record.buildNumber) {
    fail(
      `src-tauri/Info.plist OOMUBuildNumber reports ${bundleBuildNumber}; ` +
      `expected ${record.buildNumber}.`,
    );
  }
  const macosBundleVersion = tauri.bundle?.macOS?.bundleVersion;
  if (macosBundleVersion !== String(record.buildNumber)) {
    fail(
      `macOS bundleVersion reports ${JSON.stringify(macosBundleVersion)}; ` +
      `expected build ${record.buildNumber}.`,
    );
  }
  return {
    record,
    surfaces: Object.freeze({
      ...actual,
      "macOS bundleVersion": macosBundleVersion,
      "macOS build number": String(bundleBuildNumber),
    }),
  };
}

function main() {
  const { record, surfaces } = checkActiveVersionSurfaces();
  process.stdout.write(
    `OOMU version check passed: ${record.publicLabel} (${record.intendedTag}), ` +
      `build ${record.buildNumber}; ${Object.keys(surfaces).length} surfaces synchronized.\n`,
  );
}

if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(import.meta.filename)
) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
