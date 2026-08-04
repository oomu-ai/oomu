#!/usr/bin/env node

import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";
import { loadReleaseVersionRecord } from "./release-version.mjs";
import { createSanitizedChildEnvironment } from "./release-environment.mjs";
import { checksumDocument, validateReleaseNotes } from "./application-update-assets.mjs";
import { loadVerifiedApplicationUpdateCandidate } from "./application-update-candidate.mjs";
import {
  assertUpdaterPublicKeyEmbeddedInApp,
  normalizeUpdaterPublicKey,
  updaterPublicKeySha256,
  verifyUpdaterArchiveSignature,
} from "./updater-signature-verification.mjs";

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
    env: createSanitizedChildEnvironment({}, process.env),
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

function measuredAssets(paths) {
  return paths.map((path) => ({
    name: basename(path),
    sha256: sha256(path),
    sizeBytes: statSync(path).size,
  }));
}

function normalizedReleaseBody(value) {
  return String(value ?? "").replaceAll("\r\n", "\n").trim();
}

function assertExactRemoteAssets(remoteAssets, localAssets, label) {
  const expected = [...localAssets]
    .map((asset) => ({
      name: asset.name ?? basename(asset),
      sha256: asset.sha256 ?? sha256(asset),
      sizeBytes: asset.sizeBytes ?? statSync(asset).size,
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
  const received = [...(remoteAssets ?? [])]
    .map((asset) => ({
      name: asset?.name,
      digest: asset?.digest,
      sizeBytes: asset?.size ?? asset?.sizeBytes,
      state: asset?.state,
    }))
    .sort((left, right) => String(left.name).localeCompare(String(right.name)));
  if (
    expected.length !== received.length
    || expected.some((asset, index) => {
      const remote = received[index];
      return remote?.name !== asset.name
        || remote.sizeBytes !== asset.sizeBytes
        || (remote.digest != null && remote.digest !== `sha256:${asset.sha256}`)
        || (remote.state != null && remote.state !== "uploaded");
    })
  ) {
    throw new Error(`${label} does not contain exactly the verified OOMU release assets.`);
  }
  return received;
}

export function expectedPublicationAssets(version, target) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(version)) {
    throw new Error("Publication version must be plain semantic versioning.");
  }
  if (!/^darwin-(aarch64|x86_64)$/u.test(target)) throw new Error("Unsupported target.");
  const archive = `OOMU_${version}_${target}.app.tar.gz`;
  return [archive, `${archive}.sig`, "latest.json", "release-notes.json", "checksums-sha256.txt"];
}

export function validateApplicationUpdateAssetsReceipt({
  receipt,
  record,
  target,
  assets,
  candidate,
  expectedUpdaterPublicKeySha256,
}) {
  const expectedNames = expectedPublicationAssets(record.productVersion, target).sort();
  const receivedAssets = Array.isArray(receipt?.assets) ? receipt.assets : [];
  const receivedNames = receivedAssets.map((asset) => asset?.name).sort();
  if (
    receipt?.schemaVersion !== 1
    || receipt.version !== record.productVersion
    || receipt.buildNumber !== record.buildNumber
    || receipt.intendedTag !== record.intendedTag
    || receipt.target !== target
    || candidate.updaterTarget !== target
    || !/^[0-9a-f]{40}$/u.test(receipt.sourceRevision ?? "")
    || receipt.signedCandidateDescriptorPath !== candidate.descriptorPath
    || receipt.signedCandidateDescriptorSha256 !== candidate.descriptorSha256
    || receipt.qualifiedAppPath !== candidate.appPath
    || receipt.qualifiedAppTreeDigest !== candidate.appTreeDigest
    || receipt.qualifiedDmgPath !== candidate.dmgPath
    || receipt.qualifiedDmgSha256 !== candidate.dmgSha256
    || receipt.updaterPublicKeySha256 !== expectedUpdaterPublicKeySha256
    || receipt.updaterPublicKeyEmbedded !== true
    || receipt.updaterSignatureVerified !== true
    || JSON.stringify(receivedNames) !== JSON.stringify(expectedNames)
  ) {
    throw new Error("Application-update asset receipt does not match the release authority.");
  }
  const measuredByName = new Map(assets.map((asset) => [asset.name, asset]));
  for (const received of receivedAssets) {
    const measured = measuredByName.get(received.name);
    if (
      !measured
      || received.sha256 !== measured.sha256
      || received.sizeBytes !== measured.sizeBytes
    ) {
      throw new Error(`Application-update asset receipt does not match local bytes: ${received.name}`);
    }
  }
  return receipt.sourceRevision;
}

export function assertReleaseTargetCommitish(targetCommitish, sourceRevision) {
  if (!/^[0-9a-f]{40}$/u.test(sourceRevision) || targetCommitish !== sourceRevision) {
    throw new Error("GitHub release target does not match the signed application source revision.");
  }
  return sourceRevision;
}

export function assertRemoteMainRevision(remoteRevision, sourceRevision) {
  if (remoteRevision !== sourceRevision) {
    throw new Error("Remote main does not match the signed application source revision.");
  }
  return assertReleaseTargetCommitish(remoteRevision, sourceRevision);
}

export function draftReleaseCreateArguments(record, notesPath, sourceRevision) {
  assertReleaseTargetCommitish(sourceRevision, sourceRevision);
  return [
    "release", "create", record.intendedTag, "--repo", repository, "--draft",
    "--target", sourceRevision, "--title", record.publicLabel, "--notes-file", notesPath,
  ];
}

function loadLocalAssets(directory, dmgPath, record, descriptorPath, updaterPublicKey) {
  const version = record.productVersion;
  const latest = JSON.parse(readFileSync(join(directory, "latest.json"), "utf8"));
  if (latest.version !== version) throw new Error("latest.json version does not match release/version.json.");
  const targets = Object.keys(latest.platforms ?? {});
  if (targets.length !== 1) throw new Error("This publication command requires exactly one qualified target.");
  const assets = expectedPublicationAssets(version, targets[0]).map((name) => join(directory, name));
  assets.forEach((path) => {
    if (!existsSync(path) || !statSync(path).isFile()) throw new Error(`Missing update asset: ${basename(path)}`);
  });
  if (
    !existsSync(dmgPath)
    || !statSync(dmgPath).isFile()
    || basename(dmgPath) !== `OOMU-${version}.dmg`
  ) {
    throw new Error("OOMU_RELEASE_DMG_PATH must identify the qualified drag-install DMG.");
  }
  const notes = validateReleaseNotes(JSON.parse(readFileSync(join(directory, "release-notes.json"), "utf8")), version);
  const receiptPath = join(directory, "application-update-assets.receipt.json");
  if (!existsSync(receiptPath) || !statSync(receiptPath).isFile()) {
    throw new Error("Missing application-update asset receipt.");
  }
  const receipt = JSON.parse(readFileSync(receiptPath, "utf8"));
  const candidate = loadVerifiedApplicationUpdateCandidate({
    descriptorPath,
    record,
    sourceRevision: receipt.sourceRevision,
    dmgPath,
  });
  assertUpdaterPublicKeyEmbeddedInApp(candidate.appPath, updaterPublicKey);
  verifyUpdaterArchiveSignature(assets[0], assets[1], updaterPublicKey);
  const expectedArchiveUrl =
    `https://github.com/oomu-ai/oomu/releases/download/${record.intendedTag}/${basename(assets[0])}`;
  if (
    latest.platforms[targets[0]]?.signature !== readFileSync(assets[1], "utf8").trim()
    || latest.platforms[targets[0]]?.url !== expectedArchiveUrl
    || readFileSync(assets[4], "utf8") !== checksumDocument(assets.slice(0, 4))
  ) {
    throw new Error("Application-update feed metadata does not match the verified archive bytes.");
  }
  const sourceRevision = validateApplicationUpdateAssetsReceipt({
    receipt,
    record,
    target: targets[0],
    assets: measuredAssets(assets),
    candidate,
    expectedUpdaterPublicKeySha256: updaterPublicKeySha256(updaterPublicKey),
  });
  return { assets: [...assets, dmgPath], notes, sourceRevision, target: targets[0] };
}

export function validateDraftRelease(
  existing,
  record,
  sourceRevision,
  expectedBody,
) {
  if (
    !existing.isDraft
    || existing.isPrerelease
    || existing.tagName !== record.intendedTag
    || existing.name !== record.publicLabel
    || normalizedReleaseBody(existing.body) !== normalizedReleaseBody(expectedBody)
  ) {
    throw new Error("The intended release already exists outside the required public-beta draft state.");
  }
  assertReleaseTargetCommitish(existing.targetCommitish, sourceRevision);
  return existing;
}

function inspectDraft(record, sourceRevision, expectedBody) {
  const view = run(
    [
      "release", "view", record.intendedTag, "--repo", repository,
      "--json", "body,isDraft,isPrerelease,name,tagName,targetCommitish",
    ],
    "GitHub release inspection",
  );
  return validateDraftRelease(
    JSON.parse(view.stdout), record, sourceRevision, expectedBody,
  );
}

function ensureDraft(record, notesPath, sourceRevision, expectedBody) {
  const view = run(
    [
      "release", "view", record.intendedTag, "--repo", repository,
      "--json", "body,isDraft,isPrerelease,name,tagName,targetCommitish",
    ],
    "GitHub release inspection",
    true,
  );
  if (view.status === 0) {
    validateDraftRelease(
      JSON.parse(view.stdout), record, sourceRevision, expectedBody,
    );
    return;
  }
  run(
    draftReleaseCreateArguments(record, notesPath, sourceRevision),
    "draft GitHub release creation",
  );
  inspectDraft(record, sourceRevision, expectedBody);
}

function verifyRemoteMain(sourceRevision) {
  const result = run(
    ["api", `repos/${repository}/commits/main`, "--jq", ".sha"],
    "remote main source inspection",
  );
  assertRemoteMainRevision(result.stdout.trim(), sourceRevision);
}

export function assertReleaseTagRevision(tagRevision, sourceRevision) {
  if (tagRevision !== sourceRevision) {
    throw new Error("The release tag does not resolve to the signed application source revision.");
  }
  return assertReleaseTargetCommitish(tagRevision, sourceRevision);
}

function verifyRemoteReleaseTag(record, sourceRevision) {
  const result = run(
    ["api", `repos/${repository}/commits/${record.intendedTag}`, "--jq", ".sha"],
    "release tag source inspection",
  );
  return assertReleaseTagRevision(result.stdout.trim(), sourceRevision);
}

export function validateDraftPublicationState({
  draft,
  record,
  sourceRevision,
  expectedBody,
  localAssets,
}) {
  validateDraftRelease(draft, record, sourceRevision, expectedBody);
  assertExactRemoteAssets(draft.assets, localAssets, "The GitHub draft release");
  return draft;
}

function verifyDraftPublicationState(
  record,
  sourceRevision,
  expectedBody,
  localAssets,
) {
  const result = run([
    "release", "view", record.intendedTag, "--repo", repository,
    "--json", "assets,body,isDraft,isPrerelease,name,tagName,targetCommitish",
  ], "draft publication-state verification");
  return validateDraftPublicationState({
    draft: JSON.parse(result.stdout),
    record,
    sourceRevision,
    expectedBody,
    localAssets: measuredAssets(localAssets),
  });
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

export function validatePublishedLatestRelease(
  value,
  record,
  sourceRevision,
  localAssets,
  expectedBody,
) {
  if (
    value?.tag_name !== record.intendedTag
    || value?.name !== record.publicLabel
    || normalizedReleaseBody(value?.body) !== normalizedReleaseBody(expectedBody)
    || value?.draft !== false
    || value?.prerelease !== false
    || value?.target_commitish !== sourceRevision
  ) {
    throw new Error("GitHub's latest public release does not match the verified OOMU release.");
  }
  assertExactRemoteAssets(
    value.assets,
    localAssets,
    "GitHub's latest public release",
  );
  return value;
}

function verifyPublishedLatestRelease(
  record,
  sourceRevision,
  localAssets,
  expectedBody,
) {
  const result = run(
    ["api", `repos/${repository}/releases/latest`],
    "latest public release verification",
  );
  return validatePublishedLatestRelease(
    JSON.parse(result.stdout),
    record,
    sourceRevision,
    localAssets,
    expectedBody,
  );
}

export function publishApplicationUpdateRelease() {
  const record = loadReleaseVersionRecord(root);
  if (required("OOMU_CONFIRM_PUBLIC_UPDATE_RELEASE") !== `publish-${record.intendedTag}`) {
    throw new Error(`Set OOMU_CONFIRM_PUBLIC_UPDATE_RELEASE=publish-${record.intendedTag} to authorize publication.`);
  }
  const directory = realpathSync(resolve(required("OOMU_UPDATER_OUTPUT_DIR")));
  const dmgPath = realpathSync(resolve(required("OOMU_RELEASE_DMG_PATH")));
  const descriptorPath = realpathSync(resolve(required("OOMU_SIGNED_CANDIDATE_DESCRIPTOR_PATH")));
  const updaterPublicKey = normalizeUpdaterPublicKey(required("OOMU_UPDATER_PUBLIC_KEY"));
  const { assets, notes, sourceRevision, target } = loadLocalAssets(
    directory,
    dmgPath,
    record,
    descriptorPath,
    updaterPublicKey,
  );
  run(["auth", "status", "--hostname", "github.com"], "GitHub authentication check");
  verifyRemoteMain(sourceRevision);
  const temporary = mkdtempSync(join(tmpdir(), "oomu-release-notes-"));
  try {
    const expectedBody = notes.notes["en-US"];
    const notesPath = join(temporary, "release-notes.md");
    writeFileSync(notesPath, `${expectedBody}\n`, { mode: 0o600 });
    ensureDraft(record, notesPath, sourceRevision, expectedBody);
    verifyRemoteReleaseTag(record, sourceRevision);
    run(["release", "upload", record.intendedTag, "--repo", repository, "--clobber", ...assets], "draft release asset upload");
    verifyRemoteAssets(record, assets);
    verifyDraftPublicationState(record, sourceRevision, expectedBody, assets);
    verifyRemoteReleaseTag(record, sourceRevision);
    run([
      "release", "edit", record.intendedTag, "--repo", repository,
      "--draft=false", "--prerelease=false", "--latest",
    ], "atomic public release publication");
    verifyPublishedLatestRelease(
      record, sourceRevision, assets, expectedBody,
    );
    verifyRemoteReleaseTag(record, sourceRevision);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
  const receipt = {
    schemaVersion: 1,
    repository,
    tag: record.intendedTag,
    version: record.productVersion,
    sourceRevision,
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
