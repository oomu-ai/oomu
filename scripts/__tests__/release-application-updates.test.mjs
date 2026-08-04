import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  SUPPORTED_UPDATE_LOCALES,
  buildLatestManifest,
  checksumDocument,
  validateReleaseNotes,
} from "../application-update-assets.mjs";
import { expectedPublicationAssets } from "../publish-application-update-release.mjs";

const temporaryDirectories = [];

afterEach(() => {
  temporaryDirectories.splice(0).forEach((path) => rmSync(path, { recursive: true, force: true }));
});

function notes(version = "0.1.3") {
  return {
    schemaVersion: 1,
    version,
    notes: Object.fromEntries(SUPPORTED_UPDATE_LOCALES.map((locale) => [locale, `Release note for ${locale}`])),
  };
}

describe("application update release assets", () => {
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
