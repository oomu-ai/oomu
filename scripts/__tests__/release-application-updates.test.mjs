import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  SUPPORTED_UPDATE_LOCALES,
  buildLatestManifest,
  checksumDocument,
  prepareUpdaterArchiveTree,
  removeUpdaterExtraction,
  validateReleaseNotes,
} from "../application-update-assets.mjs";
import { createSanitizedChildEnvironment } from "../release-environment.mjs";
import { assertDescriptorSourceIdentity } from "../application-update-candidate.mjs";
import { assertDownloadReadmeCurrent, validateUpdaterPublicKey } from "../release.mjs";
import {
  assertRemoteMainRevision,
  assertReleaseTagRevision,
  assertReleaseTargetCommitish,
  draftReleaseCreateArguments,
  expectedPublicationAssets,
  validateApplicationUpdateAssetsReceipt,
  validateDraftPublicationState,
  validateDraftRelease,
  validatePublishedLatestRelease,
} from "../publish-application-update-release.mjs";

const temporaryDirectories = [];

afterEach(() => {
  temporaryDirectories.splice(0).forEach((path) => {
    if (existsSync(path)) removeUpdaterExtraction(path);
  });
});

function notes(version = "0.1.3") {
  return {
    schemaVersion: 1,
    version,
    notes: Object.fromEntries(SUPPORTED_UPDATE_LOCALES.map((locale) => [locale, `Release note for ${locale}`])),
  };
}

describe("application update release assets", () => {
  it("refuses a release when the prominent download page points to another version", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-download-readme-test-"));
    temporaryDirectories.push(directory);
    mkdirSync(join(directory, "downloads"));
    const version = {
      productVersion: "0.1.7",
      publicLabel: "OOMU 0.1.7 — Public Beta",
    };
    writeFileSync(
      join(directory, "downloads", "README.md"),
      "## OOMU 0.1.6 — Public Beta\n\nhttps://github.com/oomu-ai/oomu/releases/download/v0.1.6/OOMU-0.1.6.dmg\n",
    );
    expect(() => assertDownloadReadmeCurrent(directory, version, "OOMU"))
      .toThrow(/must point only to the current/u);
    writeFileSync(
      join(directory, "downloads", "README.md"),
      "## OOMU 0.1.7 — Public Beta\n\nhttps://github.com/oomu-ai/oomu/releases/download/v0.1.7/OOMU-0.1.7.dmg\n",
    );
    expect(() => assertDownloadReadmeCurrent(directory, version, "OOMU")).not.toThrow();
  });

  it("keeps Tauri updater configuration deserializable without storing a release key", () => {
    const production = JSON.parse(
      readFileSync(join(process.cwd(), "src-tauri/tauri.conf.json"), "utf8"),
    );
    const development = JSON.parse(
      readFileSync(join(process.cwd(), "src-tauri/tauri.development.conf.json"), "utf8"),
    );
    expect(production.plugins.updater).toMatchObject({
      dangerousInsecureTransportProtocol: false,
      pubkey: "",
    });
    expect(development.plugins.updater.dangerousInsecureTransportProtocol).toBe(true);
  });

  it("requires and passes only a validated production updater public key to release children", () => {
    const publicKey =
      "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyRDQ2QzMyRDZEMjIxRjMKUldUeklkTFdNbXpVWXY0L1NNM1poTzM1cTJzVHJwSi9ScHdWZ2FMandhOCttT3hFdG1WNXZpa2gK";
    expect(validateUpdaterPublicKey(` ${publicKey}\n`)).toBe(publicKey);
    expect(() => validateUpdaterPublicKey("")).toThrow(/dedicated production updater key/u);
    expect(() => validateUpdaterPublicKey("not base64!"))
      .toThrow(/dedicated production updater key/u);
    expect(() => validateUpdaterPublicKey("A".repeat(4097)))
      .toThrow(/dedicated production updater key/u);
    const child = createSanitizedChildEnvironment(
      { OOMU_UPDATER_PUBLIC_KEY: publicKey },
      { PATH: "/usr/bin", TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "private" },
    );
    expect(child.OOMU_UPDATER_PUBLIC_KEY).toBe(publicKey);
    expect(child.TAURI_SIGNING_PRIVATE_KEY_PASSWORD).toBeUndefined();
    const signer = createSanitizedChildEnvironment({
      TAURI_SIGNING_PRIVATE_KEY_PATH: "/secure/updater.key",
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "private",
    }, { PATH: "/usr/bin", TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "unscoped" });
    expect(signer.TAURI_SIGNING_PRIVATE_KEY_PATH).toBe("/secure/updater.key");
    expect(signer.TAURI_SIGNING_PRIVATE_KEY_PASSWORD).toBe("private");
    const releaseSource = readFileSync(join(process.cwd(), "scripts", "release.mjs"), "utf8");
    const sourceIdentity = readFileSync(
      join(process.cwd(), "scripts", "source-identity.mjs"), "utf8",
    );
    expect(releaseSource).toContain('requireEnvironment("OOMU_UPDATER_PUBLIC_KEY")');
    expect(releaseSource).toContain("OOMU_UPDATER_PUBLIC_KEY: updaterPublicKey");
    expect(sourceIdentity).toContain("createSanitizedChildEnvironment({}, process.env)");
  });

  it("requires exact locale and version parity", () => {
    expect(validateReleaseNotes(notes(), "0.1.3")).toEqual(notes());
    const missing = notes();
    delete missing.notes["de-DE"];
    expect(() => validateReleaseNotes(missing, "0.1.3")).toThrow(/exactly/u);
    expect(() => validateReleaseNotes(notes("0.1.4"), "0.1.3")).toThrow(/version/u);
  });

  it("constructs only the fixed GitHub release namespace", () => {
    const manifest = buildLatestManifest({
      version: "1.0.0",
      signature: "signed-archive",
      target: "darwin-aarch64",
      archiveName: "OOMU_1.0.0_darwin-aarch64.app.tar.gz",
      publishedAt: "2026-08-04T12:00:00Z",
      fallbackNote: "A reviewed update.",
    });
    expect(manifest.platforms["darwin-aarch64"].url).toBe(
      "https://github.com/oomu-ai/oomu/releases/download/v1.0.0/OOMU_1.0.0_darwin-aarch64.app.tar.gz",
    );
    expect(() => buildLatestManifest({
      ...manifest,
      version: "1.0.0-rc.1",
      signature: "signed-archive",
      target: "darwin-aarch64",
      archiveName: "OOMU_1.0.0-rc.1_darwin-aarch64.app.tar.gz",
      publishedAt: "2026-08-04T12:00:00Z",
      fallbackNote: "A reviewed update.",
    })).toThrow(/semantic/u);
  });
});

describe("application updater archive safety", () => {
  it("writes deterministic SHA-256 entries for measured bytes", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-update-assets-test-"));
    temporaryDirectories.push(directory);
    const first = join(directory, "a.txt");
    const second = join(directory, "b.txt");
    writeFileSync(first, "alpha");
    writeFileSync(second, "beta");
    const document = checksumDocument([second, first]);
    expect(document.split("\n").filter(Boolean)).toHaveLength(2);
    expect(document).toContain("a.txt");
    expect(readFileSync(first, "utf8")).toBe("alpha");
  });

  it("removes extracted signed bundles whose directories are read-only", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-update-cleanup-test-"));
    const contents = join(directory, "OOMU.app", "Contents");
    mkdirSync(contents, { recursive: true });
    writeFileSync(join(contents, "Info.plist"), "verified");
    chmodSync(contents, 0o555);
    chmodSync(join(directory, "OOMU.app"), 0o555);
    removeUpdaterExtraction(directory);
    expect(existsSync(directory)).toBe(false);
  });

  it("stages updater archives with writable directories without changing signed files", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-update-staging-test-"));
    temporaryDirectories.push(directory);
    const source = join(directory, "source", "OOMU.app");
    const contents = join(source, "Contents", "Resources");
    mkdirSync(contents, { recursive: true });
    const plist = join(source, "Contents", "Info.plist");
    writeFileSync(plist, "signed bytes");
    chmodSync(plist, 0o444);
    chmodSync(contents, 0o555);
    chmodSync(join(source, "Contents"), 0o555);
    chmodSync(source, 0o555);
    const staging = join(directory, "staging");
    mkdirSync(staging);

    const staged = prepareUpdaterArchiveTree(source, staging);

    expect(statSync(source).mode & 0o777).toBe(0o555);
    expect(statSync(staged).mode & 0o700).toBe(0o700);
    expect(statSync(join(staged, "Contents")).mode & 0o700).toBe(0o700);
    expect(statSync(join(staged, "Contents", "Resources")).mode & 0o700).toBe(0o700);
    expect(statSync(join(staged, "Contents", "Info.plist")).mode & 0o777).toBe(0o444);
    expect(readFileSync(join(staged, "Contents", "Info.plist"), "utf8")).toBe("signed bytes");
  });

  it("requires the complete updater asset set before publication", () => {
    expect(expectedPublicationAssets("0.1.3", "darwin-aarch64")).toEqual([
      "OOMU_0.1.3_darwin-aarch64.app.tar.gz",
      "OOMU_0.1.3_darwin-aarch64.app.tar.gz.sig",
      "latest.json",
      "release-notes.json",
      "checksums-sha256.txt",
    ]);
  });
});

function publicationFixture() {
  const record = {
    productVersion: "0.1.3",
    buildNumber: 8,
    intendedTag: "v0.1.3",
    publicLabel: "OOMU 0.1.3 — Public Beta",
  };
  const target = "darwin-aarch64";
  const sourceRevision = "a".repeat(40);
  const expectedUpdaterPublicKeySha256 = "c".repeat(64);
  const candidate = {
    descriptorPath: "/release/evidence/signed-candidate-descriptor.json",
    descriptorSha256: "d".repeat(64),
    appPath: "/release/candidates/OOMU.app",
    appTreeDigest: "b".repeat(64),
    dmgPath: "/release/candidates/OOMU-0.1.3.dmg",
    dmgSha256: "e".repeat(64),
    updaterTarget: target,
  };
  const assets = expectedPublicationAssets(record.productVersion, target)
    .map((name, index) => ({
      name,
      sha256: String(index + 1).repeat(64),
      sizeBytes: index + 100,
    }));
  const receipt = {
    schemaVersion: 1,
    version: record.productVersion,
    buildNumber: record.buildNumber,
    intendedTag: record.intendedTag,
    sourceRevision,
    target,
    signedCandidateDescriptorPath: candidate.descriptorPath,
    signedCandidateDescriptorSha256: candidate.descriptorSha256,
    qualifiedAppPath: candidate.appPath,
    qualifiedAppTreeDigest: candidate.appTreeDigest,
    qualifiedDmgPath: candidate.dmgPath,
    qualifiedDmgSha256: candidate.dmgSha256,
    updaterPublicKeySha256: expectedUpdaterPublicKeySha256,
    updaterPublicKeyEmbedded: true,
    updaterSignatureVerified: true,
    assets,
  };
  return {
    assets,
    candidate,
    expectedBody: "A safe public beta update.",
    expectedUpdaterPublicKeySha256,
    receipt,
    record,
    sourceRevision,
    target,
  };
}

describe("application update publication integrity", () => {
  it("binds publication to the measured updater receipt and exact source revision", () => {
    const fixture = publicationFixture();
    expect(validateApplicationUpdateAssetsReceipt({
      receipt: fixture.receipt,
      record: fixture.record,
      target: fixture.target,
      assets: fixture.assets,
      candidate: fixture.candidate,
      expectedUpdaterPublicKeySha256: fixture.expectedUpdaterPublicKeySha256,
    })).toBe(fixture.sourceRevision);
    expect(assertReleaseTargetCommitish(
      fixture.sourceRevision, fixture.sourceRevision,
    )).toBe(fixture.sourceRevision);
    expect(assertRemoteMainRevision(
      fixture.sourceRevision, fixture.sourceRevision,
    )).toBe(fixture.sourceRevision);
    expect(assertReleaseTagRevision(
      fixture.sourceRevision, fixture.sourceRevision,
    )).toBe(fixture.sourceRevision);
    expect(draftReleaseCreateArguments(
      fixture.record, "/tmp/notes.md", fixture.sourceRevision,
    )).toEqual([
      "release", "create", "v0.1.3", "--repo", "oomu-ai/oomu", "--draft",
      "--target", fixture.sourceRevision, "--title", "OOMU 0.1.3 — Public Beta",
      "--notes-file", "/tmp/notes.md",
    ]);
  });

  it("requires exact draft and public release metadata", () => {
    const fixture = publicationFixture();
    const draft = {
      isDraft: true,
      isPrerelease: false,
      tagName: fixture.record.intendedTag,
      name: fixture.record.publicLabel,
      body: fixture.expectedBody,
      targetCommitish: fixture.sourceRevision,
    };
    expect(validateDraftRelease(
      draft, fixture.record, fixture.sourceRevision, fixture.expectedBody,
    ).targetCommitish).toBe(fixture.sourceRevision);
    const publishedAssets = [
      ...fixture.assets,
      {
        name: `OOMU-${fixture.record.productVersion}.dmg`,
        sha256: "f".repeat(64),
        sizeBytes: 999,
      },
    ];
    expect(validatePublishedLatestRelease({
      tag_name: fixture.record.intendedTag,
      name: fixture.record.publicLabel,
      body: fixture.expectedBody,
      draft: false,
      prerelease: false,
      target_commitish: fixture.sourceRevision,
      assets: publishedAssets.map((asset) => ({
        name: asset.name,
        digest: `sha256:${asset.sha256}`,
        size: asset.sizeBytes,
        state: "uploaded",
      })),
    }, fixture.record, fixture.sourceRevision, publishedAssets, fixture.expectedBody).tag_name)
      .toBe(fixture.record.intendedTag);
  });
});

describe("application update publication rejection", () => {
  it("rejects release-authority and source-revision mismatches", () => {
    const fixture = publicationFixture();
    expect(() => validateApplicationUpdateAssetsReceipt({
      ...fixture,
      receipt: { ...fixture.receipt, intendedTag: "v0.1.4" },
    })).toThrow(/release authority/u);
    expect(() => validateApplicationUpdateAssetsReceipt({
      ...fixture,
      assets: fixture.assets.map((asset, index) => index === 0
        ? { ...asset, sha256: "d".repeat(64) }
        : asset),
    })).toThrow(/local bytes/u);
    expect(() => assertReleaseTargetCommitish("main", fixture.sourceRevision))
      .toThrow(/signed application source revision/u);
    expect(() => assertRemoteMainRevision("d".repeat(40), fixture.sourceRevision))
      .toThrow(/Remote main/u);
    expect(() => validateDraftRelease({
      isDraft: true,
      isPrerelease: false,
      tagName: fixture.record.intendedTag,
      name: fixture.record.publicLabel,
      body: fixture.expectedBody,
      targetCommitish: "main",
    }, fixture.record, fixture.sourceRevision, fixture.expectedBody))
      .toThrow(/source revision/u);
    expect(() => validatePublishedLatestRelease({
      tag_name: fixture.record.intendedTag,
      name: fixture.record.publicLabel,
      body: fixture.expectedBody,
      draft: false,
      prerelease: false,
      target_commitish: "main",
      assets: [],
    }, fixture.record, fixture.sourceRevision, [], fixture.expectedBody))
      .toThrow(/latest public release/u);
  });

  it("refuses stale draft assets and altered source-identity claims before publication", () => {
    const record = {
      productVersion: "0.1.3",
      buildNumber: 8,
      intendedTag: "v0.1.3",
      publicLabel: "OOMU 0.1.3 — Public Beta",
    };
    const sourceRevision = "a".repeat(40);
    const expectedBody = "A safe public beta update.";
    const localAssets = [{ name: "OOMU-0.1.3.dmg", sha256: "b".repeat(64), sizeBytes: 10 }];
    const draft = {
      isDraft: true,
      isPrerelease: false,
      tagName: record.intendedTag,
      name: record.publicLabel,
      body: expectedBody,
      targetCommitish: sourceRevision,
      assets: [
        { name: "OOMU-0.1.3.dmg", digest: `sha256:${"b".repeat(64)}`, size: 10 },
        { name: "stale-debug.zip", digest: `sha256:${"c".repeat(64)}`, size: 20 },
      ],
    };
    expect(() => validateDraftPublicationState({
      draft, record, sourceRevision, expectedBody, localAssets,
    })).toThrow(/exactly the verified/u);

    const descriptor = {
      sourceContentSha256: "d".repeat(64),
      sourceWorktreeStatusSha256: "e".repeat(64),
    };
    const currentSource = {
      sourceRevision,
      sourceContentSha256: descriptor.sourceContentSha256,
      worktreeStatusSha256: descriptor.sourceWorktreeStatusSha256,
      worktreeClean: true,
    };
    expect(assertDescriptorSourceIdentity(
      descriptor, currentSource, sourceRevision,
    )).toBe(currentSource);
    expect(() => assertDescriptorSourceIdentity(
      { ...descriptor, sourceContentSha256: "f".repeat(64) },
      currentSource,
      sourceRevision,
    )).toThrow(/current clean source/u);
  });
});
