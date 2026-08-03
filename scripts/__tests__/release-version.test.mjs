import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  assertChannelVersion,
  assertMonotonicBuildNumber,
  checkActiveVersionSurfaces,
  loadReleaseVersionRecord,
  releaseArtifactIdentifier,
  releaseDmgName,
  validateReleaseVersionRecord,
} from "../release-version.mjs";

const root = resolve(import.meta.dirname, "..", "..");
const temporaryRoots = [];

afterEach(() => {
  for (const path of temporaryRoots.splice(0)) {
    rmSync(path, { recursive: true, force: true });
  }
});

function currentRecord() {
  return JSON.parse(readFileSync(join(root, "release", "version.json"), "utf8"));
}

describe("public version authority", () => {
  it("synchronizes every active application manifest", () => {
    const result = checkActiveVersionSurfaces(root);
    expect(result.surfaces).toMatchObject({
      "package.json": "0.1.1",
      "src-tauri/tauri.conf.json": "0.1.1",
      "macOS bundleVersion": "6",
      "macOS build number": "6",
    });
  });

  it("preserves the exact first-public-beta label and intended tag contract", () => {
    const record = validateReleaseVersionRecord({
      ...currentRecord(),
      productVersion: "0.1.0",
      buildNumber: 1,
      publicLabel: "OOMU 0.1 — Public Beta",
      intendedTag: "v0.1.0",
    });
    expect(record).toMatchObject({
      productVersion: "0.1.0",
      channel: "beta",
      publicLabel: "OOMU 0.1 — Public Beta",
      intendedTag: "v0.1.0",
    });
  });

  it.each([
    ["beta", "0.1.0"],
    ["beta", "0.1.1"],
    ["nightly", "0.2.0-nightly.20260728.1"],
    ["rc", "0.2.0-rc.1"],
    ["stable", "1.0.0"],
  ])("accepts the %s version %s", (channel, version) => {
    expect(() => assertChannelVersion(version, channel)).not.toThrow();
  });

  it.each([
    ["beta", "1.257.0-beta"],
    ["nightly", "0.2.0-nightly.20260230.1"],
    ["nightly", "0.2.1-nightly.20260728.1"],
    ["nightly", "0.2.0-nightly.20260728.0"],
    ["rc", "0.2.0-rc.0"],
    ["rc", "0.2.1-rc.1"],
    ["stable", "0.9.0"],
  ])("rejects the invalid %s version %s", (channel, version) => {
    expect(() => assertChannelVersion(version, channel)).toThrow(
      /Release version contract/u,
    );
  });

  it("requires every signed-candidate build number to increase", () => {
    expect(() => assertMonotonicBuildNumber(41, 42)).not.toThrow();
    expect(() => assertMonotonicBuildNumber(42, 42)).toThrow(/greater/u);
    expect(() => assertMonotonicBuildNumber(42, 41)).toThrow(/greater/u);
  });

  it("rejects first-public-beta label drift or current tag drift", () => {
    expect(() => validateReleaseVersionRecord({
      ...currentRecord(),
      productVersion: "0.1.0",
      buildNumber: 1,
      publicLabel: "OOMU Beta",
      intendedTag: "v0.1.0",
    })).toThrow(/publicLabel must be exactly/u);
    expect(() => validateReleaseVersionRecord({
      ...currentRecord(),
      intendedTag: "v1.257.0",
    })).toThrow(/intendedTag/u);
  });

  it("derives artifact identities from version authority and build number", () => {
    const record = loadReleaseVersionRecord(root);
    expect(releaseArtifactIdentifier(record, "12345678")).toBe(
      "oomu-macos-0.1.1-build.6-12345678",
    );
    expect(releaseDmgName(record, "OOMU")).toBe("OOMU-0.1.1-build.6.dmg");
  });

  it("fails when an active manifest drifts", () => {
    const directory = mkdtempSync(join(tmpdir(), "oomu-version-drift-"));
    temporaryRoots.push(directory);
    mkdirSync(join(directory, "release"));
    mkdirSync(join(directory, "src-tauri"));
    for (const path of [
      "release/version.json",
      "package-lock.json",
      "src-tauri/Cargo.toml",
      "src-tauri/Cargo.lock",
      "src-tauri/Info.plist",
      "src-tauri/tauri.conf.json",
    ]) {
      writeFileSync(
        join(directory, path),
        readFileSync(join(root, path)),
      );
    }
    const packagePath = join(root, "package.json");
    const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
    packageJson.version = "0.1.2";
    writeFileSync(join(directory, "package.json"), `${JSON.stringify(packageJson)}\n`);
    expect(() => checkActiveVersionSurfaces(directory)).toThrow(
      /package\.json reports "0\.1\.2"; expected 0\.1\.1/u,
    );
  });
});
