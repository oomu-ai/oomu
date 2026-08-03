import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { verifyAndExpandUnsignedHandoff } from "../release-handoff.mjs";
import {
  assertToolUnchanged,
  prioritizedExecutablePath,
  releaseFailureDiagnostic,
  sha256Bytes,
} from "../release-provenance.mjs";

const root = resolve(import.meta.dirname, "..", "..");
const temporaryRoots = [];

afterEach(() => {
  for (const path of temporaryRoots.splice(0)) {
    rmSync(path, { recursive: true, force: true });
  }
});

function temporaryRoot(label) {
  const path = mkdtempSync(join(tmpdir(), `oomu-${label}-`));
  temporaryRoots.push(path);
  return path;
}

it("preserves stderr when verbose stdout exceeds the diagnostic budget", () => {
  const stdout = `stdout-first-line\n${"verbose build output\n".repeat(8_000)}stdout-final-line`;
  const diagnostic = releaseFailureDiagnostic(stdout, "exact failure reason", 1_024);

  expect(diagnostic).toContain("exact failure reason");
  expect(diagnostic).toContain("stdout-final-line");
  expect(diagnostic).not.toContain("stdout-first-line");
});

describe("immutable release provenance", () => {
  it("keeps signing and notarization out of GitHub workflows", () => {
    const policy = JSON.parse(
      readFileSync(join(root, "release", "release-policy.json"), "utf8"),
    );
    expect(existsSync(join(root, ".github", "workflows", "release.yml"))).toBe(false);
    expect(Object.values(policy.actions)).toHaveLength(4);
    expect(Object.values(policy.actions).every((sha) => /^[0-9a-f]{40}$/u.test(sha))).toBe(true);
  });

  it("binds local release tooling to the exact committed toolchain policy", () => {
    const policy = JSON.parse(
      readFileSync(join(root, "release", "release-policy.json"), "utf8"),
    );
    const rustToolchain = readFileSync(join(root, "rust-toolchain.toml"), "utf8");
    const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    expect(policy.localRunner.architecture).toBe("arm64");
    expect(policy.localRunner.xcode.developerDirectory).toBe("/Applications/Xcode.app/Contents/Developer");
    expect(prioritizedExecutablePath({
      rustup: { executable: "/shim/rustup" },
      cargo: { executable: "/pinned/cargo" },
      rustc: { executable: "/pinned/rustc" },
    }, "/pinned")).toBe("/pinned:/shim");
    expect(rustToolchain).toContain(`channel = "${policy.rust.channel}"`);
    expect(rustToolchain).toContain(`targets = ["${policy.rust.target}"]`);
    expect(packageJson.engines.node).toBe(policy.node.version);
    expect(packageJson.engines.npm).toBe(policy.node.npmVersion);
    expect(packageJson.packageManager).toBe(`npm@${policy.node.npmVersion}`);
  });

  it("keeps unsigned construction separate from protected local signing", () => {
    const unsignedSource = readFileSync(join(root, "scripts", "release-unsigned.mjs"), "utf8");
    const protectedSource = readFileSync(join(root, "scripts", "release-protected-sign.mjs"), "utf8");
    expect(unsignedSource).not.toContain("APPLE_SIGNING_IDENTITY");
    expect(protectedSource).not.toMatch(/npm ci|cargo (?:build|check|test)|tauri (?:build|bundle)/u);
    expect(protectedSource).not.toContain("release:unsigned");
    expect(protectedSource).toContain("verifyAndExpandUnsignedHandoff");
    expect(protectedSource.indexOf("verifyAndExpandUnsignedHandoff"))
      .toBeLessThan(protectedSource.indexOf('required("APPLE_SIGNING_IDENTITY")'));
  });

  it("rejects an executable whose bytes change after preflight", () => {
    const directory = temporaryRoot("release-tool-substitution");
    const executable = join(directory, "packager");
    writeFileSync(executable, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    chmodSync(executable, 0o700);
    const original = readFileSync(executable);
    const tool = {
      label: "test packager",
      executable: realpathSync(executable),
      sha256: sha256Bytes(original),
      sizeBytes: original.length,
    };

    expect(() => assertToolUnchanged(tool)).not.toThrow();
    writeFileSync(executable, "#!/bin/sh\nexit 1\n", { mode: 0o700 });
    expect(() => assertToolUnchanged(tool)).toThrow(/changed after release preflight/u);
  });

  it("rejects a mutated handoff descriptor before expansion or signing", () => {
    const directory = temporaryRoot("release-handoff-mutation");
    const handoffPath = join(directory, "unsigned-handoff.json");
    const archivePath = join(directory, "OOMU-unsigned.zip");
    writeFileSync(handoffPath, "{}\n");
    writeFileSync(archivePath, "not-an-app");
    const originalDigest = createHash("sha256")
      .update(readFileSync(handoffPath))
      .digest("hex");
    writeFileSync(handoffPath, '{"mutated":true}\n');

    expect(() => verifyAndExpandUnsignedHandoff({
      handoffPath,
      archivePath,
      expectedHandoffSha256: originalDigest,
      expectedBuildIdentifier: "build-identity-292",
      expectedSourceRevision: "a".repeat(40),
      toolchain: {},
    })).toThrow(/descriptor digest/u);
  });
});

describe("native signing authority surface", () => {
  it("does not expose either retired root-signing command", () => {
    const inspectedFiles = [
      "src-tauri/src/lib.rs",
      "src-tauri/src/command_registration.rs",
      "src-tauri/build.rs",
      "src-tauri/capabilities/default.json",
      "src-tauri/capabilities/development.json",
      "src-tauri/capabilities/production.json",
      "src/app/components/ActivityPane.tsx",
      "src/app/components/chat/directLocalFileRead.ts",
    ];
    for (const path of inspectedFiles) {
      const source = readFileSync(join(root, path), "utf8");
      expect(source).not.toMatch(/sign_artifact|sign_logical_certificate/u);
    }
    expect(existsSync(join(
      root, "src-tauri", "permissions", "autogenerated", "sign_artifact.toml",
    ))).toBe(false);
    expect(existsSync(join(
      root,
      "src-tauri",
      "permissions",
      "autogenerated",
      "sign_logical_certificate.toml",
    ))).toBe(false);
  });
});

describe("canonical OOMU icon contract", () => {
  const iconDirectory = join(root, "src-tauri", "icons");
  const appearanceExports = [
    "OOMU-macOS-ClearDark-1024x1024@1x.png",
    "OOMU-macOS-ClearLight-1024x1024@1x.png",
    "OOMU-macOS-Dark-1024x1024@1x.png",
    "OOMU-macOS-Default-1024x1024@1x.png",
    "OOMU-macOS-TintedDark-1024x1024@1x.png",
    "OOMU-macOS-TintedLight-1024x1024@1x.png",
  ];

  it("ships only the canonical icon source and every required appearance export", () => {
    expect(existsSync(join(
      iconDirectory,
      "OOMU.icon", "Assets", "OOMU_Raven_Refined.svg",
    ))).toBe(true);
    expect(existsSync(join(
      iconDirectory,
      "OOMU.icon", "Assets", "OOMU Raven.svg",
    ))).toBe(false);
    for (const exportName of appearanceExports) {
      expect(existsSync(join(iconDirectory, exportName))).toBe(true);
    }
    expect(existsSync(join(iconDirectory, "oomu-menu-raven.png"))).toBe(true);
  });

  it("binds all appearance exports as Tauri resources and uses raster web metadata", () => {
    const tauri = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    );
    for (const exportName of appearanceExports) {
      expect(tauri.bundle.resources).toContain(`icons/${exportName}`);
    }
    expect(tauri.build.beforeBundleCommand).toContain(
      "node scripts/prepare-macos-asset-catalog.mjs",
    );
    expect(tauri.bundle.macOS.files["Resources/Assets.car"]).toBe(
      "target/macos-asset-catalog/Assets.car",
    );
    expect(tauri.bundle.macOS.files["Resources/OOMU.icns"]).toBe(
      "target/macos-asset-catalog/OOMU.icns",
    );
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toMatch(/<key>CFBundleIconFile<\/key>\s*<string>OOMU<\/string>/u);
    expect(infoPlist).toMatch(/<key>CFBundleIconName<\/key>\s*<string>OOMU<\/string>/u);
    const layout = readFileSync(join(root, "src", "app", "layout.tsx"), "utf8");
    expect(layout).toContain('url: "/icon.png"');
    expect(layout).toContain('url: "/apple-icon.png"');
    expect(layout).not.toMatch(/oomu-raven\.svg/iu);
  });
});
