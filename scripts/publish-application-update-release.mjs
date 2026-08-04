#!/usr/bin/env node

import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";
import { loadReleaseVersionRecord } from "./release-version.mjs";
import { validateReleaseNotes } from "./application-update-assets.mjs";

const root = resolve(import.meta.dirname, "..");
const repository = "oomu-ai/oomu";

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

function run(args, label, allowFailure = false) {
  const result = spawnSync("/opt/homebrew/bin/gh", args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.status !== 0 && !allowFailure) {
    throw new Error(`${label} failed: ${(result.stderr || result.stdout).trim()}`);
  }
  return result;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function expectedPublicationAssets(version, target) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(version)) {
    throw new Error("Publication version must be plain semantic versioning.");
  }
  if (!/^darwin-(aarch64|x86_64)$/u.test(target)) throw new Error("Unsupported target.");
  const archive = `OOMU_${version}_${target}.app.tar.gz`;
  return [archive, `${archive}.sig`, "latest.json", "release-notes.json", "checksums-sha256.txt"];
}

function loadLocalAssets(directory, dmgPath, version) {
  const latest = JSON.parse(readFileSync(join(directory, "latest.json"), "utf8"));
  if (latest.version !== version) throw new Error("latest.json version does not match release/version.json.");
  const targets = Object.keys(latest.platforms ?? {});
  if (targets.length !== 1) throw new Error("This publication command requires exactly one qualified target.");
  const assets = expectedPublicationAssets(version, targets[0]).map((name) => join(directory, name));
  assets.forEach((path) => {
    if (!existsSync(path) || !statSync(path).isFile()) throw new Error(`Missing update asset: ${basename(path)}`);
  });
  if (!existsSync(dmgPath) || !statSync(dmgPath).isFile() || !dmgPath.endsWith(".dmg")) {
    throw new Error("OOMU_RELEASE_DMG_PATH must identify the qualified drag-install DMG.");
  }
  const notes = validateReleaseNotes(JSON.parse(readFileSync(join(directory, "release-notes.json"), "utf8")), version);
  return { assets: [...assets, dmgPath], notes, target: targets[0] };
}

function ensureDraft(record, notesPath) {
  const view = run(
    ["release", "view", record.intendedTag, "--repo", repository, "--json", "isDraft,isPrerelease"],
    "GitHub release inspection",
    true,
  );
  if (view.status === 0) {
    const existing = JSON.parse(view.stdout);
    if (!existing.isDraft || existing.isPrerelease) {
      throw new Error("The intended release already exists outside the required public-beta draft state.");
    }
    return;
  }
  run([
    "release", "create", record.intendedTag, "--repo", repository, "--draft",
    "--title", record.publicLabel, "--notes-file", notesPath,
  ], "draft GitHub release creation");
}

function verifyRemoteAssets(record, localAssets) {
  const download = mkdtempSync(join(tmpdir(), "oomu-release-readback-"));
  try {
    for (const local of localAssets) {
      run([
        "release", "download", record.intendedTag, "--repo", repository,
        "--dir", download, "--pattern", basename(local),
      ], `remote readback of ${basename(local)}`);
      const remote = join(download, basename(local));
      if (sha256(remote) !== sha256(local) || statSync(remote).size !== statSync(local).size) {
        throw new Error(`Remote release asset does not match local bytes: ${basename(local)}`);
      }
    }
  } finally {
    rmSync(download, { recursive: true, force: true });
  }
}

export function publishApplicationUpdateRelease() {
  const record = loadReleaseVersionRecord(root);
  if (required("OOMU_CONFIRM_PUBLIC_UPDATE_RELEASE") !== `publish-${record.intendedTag}`) {
    throw new Error(`Set OOMU_CONFIRM_PUBLIC_UPDATE_RELEASE=publish-${record.intendedTag} to authorize publication.`);
  }
  const directory = realpathSync(resolve(required("OOMU_UPDATER_OUTPUT_DIR")));
  const dmgPath = realpathSync(resolve(required("OOMU_RELEASE_DMG_PATH")));
  const { assets, notes, target } = loadLocalAssets(directory, dmgPath, record.productVersion);
  run(["auth", "status", "--hostname", "github.com"], "GitHub authentication check");
  const temporary = mkdtempSync(join(tmpdir(), "oomu-release-notes-"));
  try {
    const notesPath = join(temporary, "release-notes.md");
    writeFileSync(notesPath, `${notes.notes["en-US"]}\n`, { mode: 0o600 });
    ensureDraft(record, notesPath);
    run(["release", "upload", record.intendedTag, "--repo", repository, "--clobber", ...assets], "draft release asset upload");
    verifyRemoteAssets(record, assets);
    run([
      "release", "edit", record.intendedTag, "--repo", repository,
      "--draft=false", "--prerelease=false", "--latest",
    ], "atomic public release publication");
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
  const receipt = {
    schemaVersion: 1,
    repository,
    tag: record.intendedTag,
    version: record.productVersion,
    target,
    publishedAt: new Date().toISOString(),
    assets: assets.map((path) => ({ name: basename(path), sha256: sha256(path), sizeBytes: statSync(path).size })),
  };
  const receiptPath = join(directory, "application-update-publication.receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o444 });
  return receiptPath;
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  try {
    process.stdout.write(`Published and remotely verified: ${publishApplicationUpdateRelease()}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
