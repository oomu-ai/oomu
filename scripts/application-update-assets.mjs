#!/usr/bin/env node

import {
  chmodSync,
  cpSync,
  existsSync,
  lstatSync,
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
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";
import { artifactDigestForEntries, collectTreeEntries } from "./release-manifest.mjs";
import { loadReleaseVersionRecord } from "./release-version.mjs";
import { loadVerifiedApplicationUpdateCandidate } from "./application-update-candidate.mjs";
import { createSanitizedChildEnvironment } from "./release-environment.mjs";
import {
  assertUpdaterPublicKeyEmbeddedInApp,
  normalizeUpdaterPublicKey,
  updaterPublicKeySha256,
  verifyUpdaterArchiveSignature,
} from "./updater-signature-verification.mjs";

export const SUPPORTED_UPDATE_LOCALES = Object.freeze([
  "de-DE", "en-US", "es-ES", "fr-FR", "id-ID", "ja-JP",
  "pt-BR", "ru-RU", "uk-UA", "vi-VN", "zh-CN", "zh-TW",
]);

const root = resolve(import.meta.dirname, "..");
const MAX_NOTES_BYTES = 64 * 1024;
const PLAIN_SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

function run(executable, args, options = {}) {
  const result = spawnSync(executable, args, {
    cwd: options.cwd ?? root,
    encoding: "utf8",
    env: options.env ?? createSanitizedChildEnvironment({}, process.env),
    maxBuffer: 8 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(`${options.label ?? basename(executable)} failed: ${(result.stderr || result.stdout).trim()}`);
  }
  return result.stdout.trim();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8", mode: 0o444 });
}

function assertPlainSemver(value) {
  if (!PLAIN_SEMVER.test(value)) throw new Error(`Update version is not plain semantic versioning: ${value}`);
}

export function validateReleaseNotes(document, expectedVersion) {
  if (!document || typeof document !== "object" || Array.isArray(document)) {
    throw new Error("release-notes.json must contain one object.");
  }
  if (document.schemaVersion !== 1 || document.version !== expectedVersion) {
    throw new Error("Release-note schema or version does not match the release authority.");
  }
  const locales = Object.keys(document.notes ?? {}).sort();
  if (JSON.stringify(locales) !== JSON.stringify([...SUPPORTED_UPDATE_LOCALES].sort())) {
    throw new Error("Release notes must contain exactly the supported OOMU locales.");
  }
  for (const locale of locales) {
    const note = document.notes[locale];
    if (typeof note !== "string" || !note.trim() || note.length > 12_000 || /[\u0000-\u0008\u000B\u000C\u000E-\u001F]/u.test(note)) {
      throw new Error(`Release notes for ${locale} are invalid.`);
    }
  }
  const bytes = Buffer.from(`${JSON.stringify(document, null, 2)}\n`, "utf8");
  if (bytes.byteLength > MAX_NOTES_BYTES) throw new Error("Localized release notes exceed 64 KiB.");
  return document;
}

export function buildLatestManifest({ version, signature, target, archiveName, publishedAt, fallbackNote }) {
  assertPlainSemver(version);
  if (!/^darwin-(aarch64|x86_64)$/u.test(target)) throw new Error(`Unsupported update target: ${target}`);
  if (!signature?.trim() || signature.length > 4096) throw new Error("Updater signature is missing or too large.");
  if (typeof fallbackNote !== "string" || !fallbackNote.trim()) throw new Error("The reviewed English fallback note is required.");
  if (Number.isNaN(Date.parse(publishedAt))) throw new Error("OOMU_UPDATE_PUBLISHED_AT must be an ISO timestamp.");
  const expectedName = `OOMU_${version}_${target}.app.tar.gz`;
  if (archiveName !== expectedName) throw new Error("Updater archive name does not match its version and target.");
  return {
    version,
    notes: fallbackNote.trim(),
    pub_date: new Date(publishedAt).toISOString(),
    platforms: {
      [target]: {
        signature: signature.trim(),
        url: `https://github.com/oomu-ai/oomu/releases/download/v${version}/${archiveName}`,
      },
    },
  };
}

export function checksumDocument(paths) {
  return paths
    .map((path) => `${sha256(path)}  ${basename(path)}`)
    .sort((left, right) => left.localeCompare(right))
    .join("\n") + "\n";
}

function assertQualifiedApp(appPath) {
  const app = realpathSync(resolve(appPath));
  if (!lstatSync(app).isDirectory() || basename(app) !== "OOMU.app") {
    throw new Error("OOMU_QUALIFIED_APP_PATH must identify the qualified OOMU.app bundle.");
  }
  run("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=2", app], { label: "codesign verification" });
  run("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=2", app], { label: "Gatekeeper assessment" });
  run("/usr/bin/xcrun", ["stapler", "validate", app], { label: "notarization staple validation" });
  return app;
}

function prepareOutput(path) {
  const destination = resolve(path);
  if (existsSync(destination)) throw new Error("OOMU_UPDATER_OUTPUT_DIR must not already exist.");
  const staging = `${destination}.staging-${process.pid}-${Date.now()}`;
  if (existsSync(staging)) throw new Error("Updater staging directory already exists.");
  mkdirSync(staging, { recursive: false, mode: 0o700 });
  return { destination, staging };
}

export function removeUpdaterExtraction(path) {
  run("/bin/chmod", ["-R", "u+w", path], {
    env: createSanitizedChildEnvironment({}, {}),
    label: "updater extraction cleanup",
  });
  rmSync(path, { recursive: true, force: true });
}

export function prepareUpdaterArchiveTree(sourceApp, stagingRoot) {
  const stagedApp = join(stagingRoot, "OOMU.app");
  cpSync(sourceApp, stagedApp, {
    recursive: true,
    preserveTimestamps: true,
    verbatimSymlinks: true,
  });
  const makeDirectoriesExtractable = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const child = join(directory, entry.name);
      makeDirectoriesExtractable(child);
      chmodSync(child, statSync(child).mode | 0o700);
    }
  };
  makeDirectoriesExtractable(stagedApp);
  chmodSync(stagedApp, statSync(stagedApp).mode | 0o700);
  return stagedApp;
}

function archiveQualifiedApp(app, archivePath) {
  const archiveStaging = mkdtempSync(join(tmpdir(), "oomu-update-staging-"));
  const extraction = mkdtempSync(join(tmpdir(), "oomu-update-archive-"));
  try {
    const stagedApp = prepareUpdaterArchiveTree(app, archiveStaging);
    run("/usr/bin/tar", ["-czf", archivePath, "-C", dirname(stagedApp), basename(stagedApp)], {
      label: "updater archive creation",
      env: {
        ...createSanitizedChildEnvironment({}, process.env),
        COPYFILE_DISABLE: "1",
      },
    });
    run("/usr/bin/tar", ["-xzf", archivePath, "-C", extraction], { label: "updater archive extraction" });
    const extracted = join(extraction, "OOMU.app");
    const sourceDigest = artifactDigestForEntries(collectTreeEntries(app));
    const extractedDigest = artifactDigestForEntries(collectTreeEntries(extracted));
    if (sourceDigest !== extractedDigest) throw new Error("Updater archive does not reproduce the qualified app tree.");
    run("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=2", extracted], { label: "extracted app signature verification" });
    return sourceDigest;
  } finally {
    removeUpdaterExtraction(archiveStaging);
    removeUpdaterExtraction(extraction);
  }
}

function signArchive(archivePath, updaterPublicKey) {
  required("TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY && !process.env.TAURI_SIGNING_PRIVATE_KEY_PATH) {
    throw new Error("A dedicated Tauri updater private key is required.");
  }
  const cli = join(root, "node_modules", ".bin", "tauri");
  const signingEnvironment = createSanitizedChildEnvironment(
    Object.fromEntries([
      "TAURI_SIGNING_PRIVATE_KEY",
      "TAURI_SIGNING_PRIVATE_KEY_PATH",
      "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    ]
      .filter((name) => process.env[name] !== undefined)
      .map((name) => [name, process.env[name]])),
    process.env,
  );
  run(cli, ["signer", "sign", archivePath], {
    label: "Tauri updater signature",
    env: signingEnvironment,
  });
  const signaturePath = `${archivePath}.sig`;
  if (!existsSync(signaturePath) || statSync(signaturePath).size === 0) {
    throw new Error("Tauri did not produce the updater signature.");
  }
  verifyUpdaterArchiveSignature(archivePath, signaturePath, updaterPublicKey);
  return signaturePath;
}

function assertCleanSource() {
  const status = run("/usr/bin/git", ["status", "--porcelain=v1", "--untracked-files=all"], { label: "source cleanliness check" });
  if (status) throw new Error("Updater artifacts require a clean, committed source tree.");
  return run("/usr/bin/git", ["rev-parse", "HEAD"], { label: "source revision" });
}

export function createApplicationUpdateAssets() {
  const versionRecord = loadReleaseVersionRecord(root);
  assertPlainSemver(versionRecord.productVersion);
  const updaterPublicKey = normalizeUpdaterPublicKey(required("OOMU_UPDATER_PUBLIC_KEY"));
  const sourceRevision = assertCleanSource();
  const app = assertQualifiedApp(required("OOMU_QUALIFIED_APP_PATH"));
  const candidate = loadVerifiedApplicationUpdateCandidate({
    descriptorPath: required("OOMU_SIGNED_CANDIDATE_DESCRIPTOR_PATH"),
    record: versionRecord,
    sourceRevision,
    appPath: app,
  });
  assertUpdaterPublicKeyEmbeddedInApp(app, updaterPublicKey);
  const { destination, staging } = prepareOutput(required("OOMU_UPDATER_OUTPUT_DIR"));
  try {
    populateUpdateAssets({
      output: staging,
      app,
      sourceRevision,
      updaterPublicKey,
      versionRecord,
      candidate,
    });
    renameSync(staging, destination);
    return {
      output: destination,
      receiptPath: join(destination, "application-update-assets.receipt.json"),
    };
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    throw error;
  }
}

function populateUpdateAssets({
  output,
  app,
  sourceRevision,
  updaterPublicKey,
  versionRecord,
  candidate,
}) {
  const target = candidate.updaterTarget;
  const archiveName = `OOMU_${versionRecord.productVersion}_${target}.app.tar.gz`;
  const archivePath = join(output, archiveName);
  const appTreeDigest = archiveQualifiedApp(app, archivePath);
  if (appTreeDigest !== candidate.appTreeDigest) {
    throw new Error("Updater archive source does not match the descriptor-bound application.");
  }
  const signaturePath = signArchive(archivePath, updaterPublicKey);
  const signature = readFileSync(signaturePath, "utf8").trim();
  const notesSource = join(
    root,
    "release",
    "update-notes",
    `${versionRecord.productVersion}.json`,
  );
  const notes = validateReleaseNotes(JSON.parse(readFileSync(notesSource, "utf8")), versionRecord.productVersion);
  const notesPath = join(output, "release-notes.json");
  writeJson(notesPath, notes);
  const manifestPath = join(output, "latest.json");
  writeJson(manifestPath, buildLatestManifest({
    version: versionRecord.productVersion,
    signature,
    target,
    archiveName,
    publishedAt: required("OOMU_UPDATE_PUBLISHED_AT"),
    fallbackNote: notes.notes["en-US"],
  }));
  const checksumPath = join(output, "checksums-sha256.txt");
  writeFileSync(checksumPath, checksumDocument([archivePath, signaturePath, notesPath, manifestPath]), { encoding: "utf8", mode: 0o444 });
  const receiptPath = join(output, "application-update-assets.receipt.json");
  writeJson(receiptPath, {
    schemaVersion: 1,
    version: versionRecord.productVersion,
    buildNumber: versionRecord.buildNumber,
    intendedTag: versionRecord.intendedTag,
    sourceRevision,
    target,
    signedCandidateDescriptorPath: candidate.descriptorPath,
    signedCandidateDescriptorSha256: candidate.descriptorSha256,
    qualifiedAppPath: candidate.appPath,
    qualifiedAppTreeDigest: appTreeDigest,
    qualifiedDmgPath: candidate.dmgPath,
    qualifiedDmgSha256: candidate.dmgSha256,
    updaterPublicKeySha256: updaterPublicKeySha256(updaterPublicKey),
    updaterPublicKeyEmbedded: true,
    updaterSignatureVerified: true,
    assets: readdirSync(output).sort().map((name) => ({ name, sha256: sha256(join(output, name)), sizeBytes: statSync(join(output, name)).size })),
  });
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    const result = createApplicationUpdateAssets();
    process.stdout.write(`Verified application-update assets created at ${result.output}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
