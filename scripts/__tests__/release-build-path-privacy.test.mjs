import { mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  inspectBuildPathPrivacy,
  run,
} from "../release-gates/build-path-privacy.mjs";
import {
  assertNoReleaseEnvironmentOverrides,
  canonicalNativePathRemapConfiguration,
  canonicalNativePathRemapEnvironment,
  releaseToolchainHomeDirectory,
} from "../release-environment.mjs";
import { inspectNativePathRemapArtifact } from "../preflight-native-path-remap.mjs";

const root = resolve(import.meta.dirname, "..", "..");

function fixture() {
  const repository = mkdtempSync(join(tmpdir(), "oomu-path-privacy-repository-"));
  const app = join(repository, "candidate", "OOMU.app");
  const macOS = join(app, "Contents", "MacOS");
  const resources = join(app, "Contents", "Resources");
  mkdirSync(macOS, { recursive: true });
  mkdirSync(resources, { recursive: true });
  return { app, macOS, repository, resources };
}

function fakeMachO(content) {
  return Buffer.concat([
    Buffer.from("cffaedfe", "hex"),
    Buffer.alloc(32),
    Buffer.from(content, "utf8"),
    Buffer.from([0]),
  ]);
}

describe("release build-path privacy", () => {
  it("uses stable path remapping only through the reviewed production environment", () => {
    const environment = canonicalNativePathRemapEnvironment(
      "/Users/release-builder/work/OOMU",
      { HOME: "/Users/release-builder" },
      "/Users/release-builder",
    );
    const flags = environment.CARGO_ENCODED_RUSTFLAGS.split("\u001f");
    expect(flags).toContain("--remap-path-scope=all");
    expect(flags).toContain(
      "--remap-path-prefix=/Users/release-builder/work/OOMU=/oomu/source",
    );
    expect(flags).toContain(
      "--remap-path-prefix=/Users/release-builder/.cargo=/oomu/toolchains/cargo",
    );
    expect(flags).toContain(
      "--remap-path-prefix=/Users/release-builder/.rustup=/oomu/toolchains/rustup",
    );
    expect(flags).toContain(
      "--remap-path-prefix=/Users/release-builder=/oomu/builder-home",
    );
    for (const name of ["CFLAGS", "CXXFLAGS"]) {
      expect(environment[name]).toContain(
        "-ffile-prefix-map=/Users/release-builder/work/OOMU=/oomu/source",
      );
      expect(environment[name]).toContain(
        "-fdebug-prefix-map=/Users/release-builder/.cargo=/oomu/toolchains/cargo",
      );
      expect(environment[name]).toContain(
        "-fmacro-prefix-map=/Users/release-builder=/oomu/builder-home",
      );
    }
    expect(() => assertNoReleaseEnvironmentOverrides({
      CARGO_ENCODED_RUSTFLAGS: "--unreviewed",
    })).toThrow(/environment overrides are prohibited/i);
    expect(() => assertNoReleaseEnvironmentOverrides({
      CARGO_HOME: "/unreviewed/cargo",
    })).toThrow(/environment overrides are prohibited/i);
    expect(() => assertNoReleaseEnvironmentOverrides({
      CXXFLAGS: "-DUNREVIEWED_RELEASE_INPUT=1",
    })).toThrow(/environment overrides are prohibited/i);

    const canonicalRelease = readFileSync(join(root, "scripts", "release.mjs"), "utf8");
    const unsignedRelease = readFileSync(
      join(root, "scripts", "release-unsigned.mjs"), "utf8",
    );
    const canonicalEvidence = readFileSync(
      join(root, "scripts", "release-canonical-evidence.mjs"), "utf8",
    );
    const packageJson = readFileSync(join(root, "package.json"), "utf8");
    expect(canonicalRelease).toContain("canonicalNativePathRemapEnvironment(");
    expect(unsignedRelease).toContain("canonicalNativePathRemapEnvironment(");
    expect(canonicalRelease).toContain("releaseToolchainHomeDirectory(immutableReleaseToolchain)");
    expect(unsignedRelease).toContain("releaseToolchainHomeDirectory(toolchain)");
    expect(canonicalRelease).toContain('runStep("automated_native_path_remap"');
    expect(canonicalRelease).toContain("nativePathRemap.checked_languages");
    expect(canonicalEvidence).toContain("native_path_remap: context.nativePathRemap");
    expect(canonicalRelease.indexOf('runStep("automated_native_path_remap"')).toBeLessThan(
      canonicalRelease.indexOf("...runRustQualification(node, releaseEnvironment)"),
    );
    expect(packageJson).not.toContain("CARGO_ENCODED_RUSTFLAGS");
    expect(releaseToolchainHomeDirectory({
      tools: {
        cargo: {
          executable:
            "/Users/release-builder/.rustup/toolchains/1.91.0-aarch64-apple-darwin/bin/cargo",
        },
      },
    })).toBe("/Users/release-builder");
    expect(() => canonicalNativePathRemapEnvironment(
      "/Users/release-builder/work/OOMU",
      { HOME: "/Users/other-builder" },
      "/Users/release-builder",
    )).toThrow(/does not match the pinned Rust toolchain home/i);
    expect(() => canonicalNativePathRemapEnvironment(
      "/Users/release builder/work/OOMU",
      { HOME: "/Users/release builder" },
      "/Users/release builder",
    )).toThrow(/bounded absolute paths/i);
  });

  it("rejects local C and C++ canary paths before the production compile", () => {
    const configuration = canonicalNativePathRemapConfiguration(
      "/Users/release-builder/work/OOMU",
      { HOME: "/Users/release-builder" },
    );
    expect(inspectNativePathRemapArtifact(
      Buffer.from("/Users/release-builder/.cargo/registry/native.cpp"),
      configuration,
    ).findings).toEqual(["local_path_marker"]);
    expect(inspectNativePathRemapArtifact(
      Buffer.from("/oomu/toolchains/cargo/registry/native.cpp"),
      configuration,
    ).findings).toEqual([]);
  });
});

describe("release build-path privacy bundle scan", () => {
  it("accepts canonical production paths and ordinary bundle resources", async () => {
    const value = fixture();
    writeFileSync(join(value.macOS, "oomu"), fakeMachO("/oomu/source/src/main.rs"));
    writeFileSync(join(value.resources, "settings.json"), "{\"ready\":true}\n");
    await expect(run({ root: value.repository, appPath: value.app })).resolves.toMatchObject({
      passed: true,
      evidence: { findings: [], mach_o_files_scanned: 1 },
    });
  });

  it("rejects and redacts a builder home path embedded in Mach-O bytes", () => {
    const value = fixture();
    writeFileSync(
      join(value.macOS, "oomu"),
      fakeMachO("panic at /Users/private-builder/.cargo/registry/src/lib.rs"),
    );
    const inspection = inspectBuildPathPrivacy(value.app, {
      repositoryRoot: value.repository,
      homeDirectory: "/Users/private-builder",
    });
    expect(inspection.findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        path: "Contents/MacOS/oomu",
        kind: "mach_o",
        rule: "absolute_macos_user_path",
      }),
    ]));
    expect(JSON.stringify(inspection.findings)).not.toContain("private-builder");
  });

  it("rejects exact repository paths, data-volume user paths, and absolute symlink targets", () => {
    const value = fixture();
    writeFileSync(join(value.macOS, "oomu"), fakeMachO("clean"));
    writeFileSync(
      join(value.resources, "metadata.txt"),
      `${value.repository}/source.rs\n/System/Volumes/Data/Users/builder/output\n`,
    );
    symlinkSync("/Users/release-builder/private.dat", join(value.resources, "private-link"));
    const inspection = inspectBuildPathPrivacy(value.app, {
      repositoryRoot: value.repository,
      homeDirectory: "/Users/release-builder",
    });
    expect(inspection.findings.map(({ rule }) => rule)).toEqual(expect.arrayContaining([
      "exact_repository_build_path",
      "absolute_macos_data_user_path",
      "absolute_macos_user_path",
    ]));
  });
});
