import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const runnerPath = join(root, "scripts", "tauri-dev-runner.mjs");
const runner = readFileSync(runnerPath, "utf8");
const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

describe("Tauri development macOS permission identity", () => {
  it("runs every Tauri development launch through the isolated signer", () => {
    const developmentPhases = packageJson.scripts.dev.split(" && ");
    expect(packageJson.scripts.dev).toContain(
      "tauri dev --runner ../scripts/tauri-dev-runner.mjs",
    );
    expect(statSync(runnerPath).mode & 0o111).not.toBe(0);
    for (const preparation of [
      "prepare-portable-python.mjs",
      "prepare-rust-helpers.mjs",
      "prepare-artifact-pdf-helper.mjs",
      "prepare-vision-helper.mjs",
      "prepare-tauri-external-bins.mjs --verify",
    ]) {
      expect(packageJson.scripts.dev).toContain(preparation);
    }
    for (const supersededPreparation of [
      "prepare-local-infer.mjs",
      "prepare-pdf-helper.mjs",
      "prepare-artifact-helper.mjs",
    ]) {
      expect(packageJson.scripts.dev).not.toContain(supersededPreparation);
    }
    expect(developmentPhases.slice(1, 6)).toEqual([
      "node scripts/prepare-rust-helpers.mjs",
      "node scripts/prepare-artifact-pdf-helper.mjs",
      "node scripts/prepare-vision-helper.mjs",
      "src-tauri/src-swift/build.sh",
      "node scripts/prepare-tauri-external-bins.mjs --verify",
    ]);
  });

  it("uses the development identifier and reviewed entitlements without password access", () => {
    expect(runner).toContain('"ai.eldris.oomu.gpd.development"');
    expect(runner).toContain('"--sign", signingIdentity');
    expect(runner).toContain('join(repositoryRoot, "src-tauri", "entitlements.plist")');
    expect(runner).toContain('["find-identity", "-v", "-p", "codesigning"]');
    expect(runner).not.toContain("find-generic-password");
    expect(runner).not.toContain("APPLE_SIGNING_IDENTITY");
  });

  it("prefers one Apple Development identity and otherwise fails safely to ad hoc", () => {
    expect(runner).toContain("OOMU_TAURI_DEV_SIGNING_IDENTITY");
    expect(runner).toContain('identity.startsWith("Apple Development:")');
    expect(runner).toContain('return identities.length === 1 ? identities[0] : "-"');
  });

  it("launches from a real isolated app bundle with existing localized privacy copy", () => {
    expect(runner).toContain('"development-bundle", "OOMU Development.app"');
    expect(runner).toContain('copyFileSync(sourceInfoPlistPath, bundledInfoPlistPath)');
    expect(runner).toContain('cpSync(sourceLocalizationsPath, resourcesPath, { recursive: true })');
    expect(runner).toContain('["CFBundleIdentifier", "string", DEVELOPMENT_IDENTIFIER]');
    expect(runner).toContain('launchPath = bundledExecutablePath');
    expect(runner).toContain('spawn(\n      "/usr/bin/open"');
    expect(runner).toContain('["-n", "-W", launchBundlePath');
    expect(runner).toContain('["-f", `^${launchPath}$`]');
    expect(runner).toContain('fail("An OOMU Development instance is already running.")');
  });

  it("mirrors every configured helper and runtime resource into the development app", () => {
    expect(runner).toContain("for (const configuredPath of tauriConfig.bundle.externalBin)");
    expect(runner).toContain('const destinationPath = join(macosPath, basename(configuredPath))');
    expect(runner).toContain('join(resourcesPath, "resources")');
    expect(runner).toContain('join(resourcesPath, "_up_", "src", "locales")');
    expect(runner).toContain('join(resourcesPath, "_up_", "THIRD_PARTY_NOTICES.md")');
  });

  it("refuses to execute anything outside the Cargo target directory", () => {
    expect(runner).toContain('runnerArgs[0] !== "run"');
    expect(runner).toContain('const cargoBuildArgs = ["build", ...cargoRunArgs.slice(1)]');
    expect(runner).toContain("binaryPath.startsWith(`${realpathSync(targetRoot)}${sep}`)");
    expect(runner).toContain('basename(binaryPath) !== "oomu"');
  });
});
