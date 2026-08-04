import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { generateKeyPairSync } from "node:crypto";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  verifyFinalSignedEntitlements,
  verifyEntitlementSnapshot,
  verifySignedEntitlements,
} from "../check-entitlements.mjs";
import {
  assertAppleDeveloperToolPath,
  assertTrustedAppleToolPath,
} from "../preflight-apple-toolchain.mjs";
import {
  assertExpectedReleaseArchitecture,
  assertNoReleaseEnvironmentOverrides,
  assertNoRepositoryDotenvFiles,
  createExclusiveReleaseOutputDirectories,
  createReleaseAuthorization,
  externalHarnessEnvironment,
  sanitizedChildEnvironment,
} from "../release.mjs";
import { verifyReleaseAuthorization } from "../assert-release-entrypoint.mjs";
import { TRUSTED_RELEASE_PUBLIC_KEY_HEX } from "../release-manifest.mjs";
import {
  SUPPORTED_LOCALE_FILES,
  validateConfiguredLocaleResources,
} from "../release-gates/bundle-resource-inventory.mjs";

const root = resolve(import.meta.dirname, "..", "..");
const temporaryRoots = [];
const RELEASE_AUTHORIZATION_ENV_NAMES = [
  "OOMU_LOCAL_UNSIGNED_BUILD",
  "OOMU_RELEASE_PIPELINE",
  "OOMU_BUILD_ID",
  "OOMU_SOURCE_REVISION",
  "OOMU_RELEASE_POLICY_SHA256",
  "OOMU_RELEASE_AUTHORIZATION_BASE64",
  "OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH",
];

function expectContainsEach(source, markers) {
  for (const marker of markers) expect(source).toContain(marker);
}

function expectLocalSigningBoundary() {
  const signer = readFileSync(join(root, "scripts", "sign-local-macos-app.mjs"), "utf8");
  const policy = readFileSync(join(root, "scripts", "local-signing-identity.mjs"), "utf8");
  expect(signer).toContain("localSigningIdentityPolicy");
  expect(signer).toContain("OOMU_LOCAL_APP_PATH");
  expect(signer).toContain("src-tauri/target");
  expect(signer).toContain('"--verify", "--deep", "--strict"');
  expect(policy).toContain('"ai.eldris.oomu.gpd"');
  expect(policy).toContain('"ai.eldris.oomu.gpd.development"');
}

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

function withoutEnvironmentKeys(source, names) {
  const environment = { ...source };
  for (const name of names) delete environment[name];
  return environment;
}

describe("production release entrypoints", () => {
  it("keeps macOS bundle versions numeric and disables obsolete launch metadata", () => {
    const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    const tauri = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    );
    const releaseVersion = JSON.parse(
      readFileSync(join(root, "release", "version.json"), "utf8"),
    );
    const packageVersion = packageJson.version.match(
      /^(\d+\.\d+\.\d+)(?:-[0-9A-Za-z.-]+)?$/,
    );

    expect(packageVersion).not.toBeNull();
    expect(tauri.version).toMatch(/^\d+\.\d+\.\d+$/);
    expect(tauri.bundle.macOS.bundleVersion).toMatch(/^\d+$/);
    expect(tauri.version).toBe(packageVersion?.[1]);
    expect(tauri.bundle.macOS.bundleVersion).toBe(String(releaseVersion.buildNumber));

    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toMatch(
      new RegExp(
        `<key>OOMUBuildNumber</key>\\s*<integer>${releaseVersion.buildNumber}</integer>`,
        "u",
      ),
    );
    for (const key of ["CSResourcesFileMapped", "LSRequiresCarbon"]) {
      const keyPattern = new RegExp(`<key>${key}</key>`);
      const enabledPattern = new RegExp(`<key>${key}</key>\\s*<true\\s*/>`);
      const disabledPattern = new RegExp(`<key>${key}</key>\\s*<false\\s*/>`);
      expect(enabledPattern.test(infoPlist)).toBe(false);
      expect(!keyPattern.test(infoPlist) || disabledPattern.test(infoPlist)).toBe(true);
    }
    expect(infoPlist).toMatch(
      /<key>LSMultipleInstancesProhibited<\/key>\s*<true\s*\/>/,
    );
  });

  it("keeps development permissions isolated from the production app identity", () => {
    const production = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    );
    const development = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.development.conf.json"), "utf8"),
    );

    expect(development.identifier).toBe("ai.eldris.oomu.gpd.development");
    expect(development.identifier).not.toBe(production.identifier);
    expect(development.productName).toBe("OOMU Development");
  });

  it("packages exactly the 12 supported locale JSON catalogs and no test source", () => {
    const tauri = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    );
    expect(validateConfiguredLocaleResources(tauri)).toEqual(
      SUPPORTED_LOCALE_FILES.map((file) => `../src/locales/${file}`).sort(),
    );
    for (const file of SUPPORTED_LOCALE_FILES) {
      expect(() => JSON.parse(readFileSync(join(root, "src", "locales", file), "utf8")))
        .not.toThrow();
    }

    const contaminated = structuredClone(tauri);
    contaminated.bundle.resources.push("../src/locales/englishTranslationSource.test.ts");
    expect(() => validateConfiguredLocaleResources(contaminated)).toThrow(
      /exactly the 12 supported JSON catalogs/iu,
    );
  });

  it("packages localized Calendar Full Access privacy copy for every supported locale", () => {
    const expectedPurpose =
      "OOMU checks your schedule for conflicts and creates only the Calendar events you approve.";
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toContain(`<string>${expectedPurpose}</string>`);

    const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
    const localeFolders = [
      "de", "en", "es", "fr", "id", "ja", "pt-BR", "ru", "uk", "vi", "zh-Hans", "zh-Hant",
    ];
    const expectedFiles = {
      "Resources/Assets.car": "target/macos-asset-catalog/Assets.car",
      "Resources/OOMU.icns": "target/macos-asset-catalog/OOMU.icns",
    };
    for (const locale of localeFolders) {
      const source = `macos-localizations/${locale}.lproj/InfoPlist.strings`;
      const destination = `Resources/${locale}.lproj/InfoPlist.strings`;
      expectedFiles[destination] = source;
      expect(tauri.bundle.macOS.files[destination]).toBe(source);
      const localized = readFileSync(join(root, "src-tauri", source), "utf8");
      expect(localized.match(/"NSCalendarsUsageDescription"\s*=\s*"[^"]+";/g)).toHaveLength(1);
      expect(localized.match(/"NSCalendarsFullAccessUsageDescription"\s*=\s*"[^"]+";/g)).toHaveLength(1);
      expect(localized.match(/"NSAppleEventsUsageDescription"\s*=\s*"[^"]+";/g)).toHaveLength(1);
    }
    expect(tauri.bundle.macOS.files).toEqual(expectedFiles);
  });

  it("declares user-approved Apple Events access for Apple app integrations", () => {
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toContain("<key>NSAppleEventsUsageDescription</key>");
    expect(infoPlist).toContain(
      "<string>OOMU works with Mail, Notes, Reminders, Messages, and other Apple apps only when you ask.</string>",
    );
    expect(verifyEntitlementSnapshot().entitlements).toMatchObject({
      "com.apple.security.automation.apple-events": true,
      "com.apple.security.personal-information.addressbook": true,
      "com.apple.security.personal-information.calendars": true,
    });
  });

});

describe("production release privacy and native capabilities", () => {

  it("declares native Photos access on the main OOMU bundle", () => {
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toContain("<key>NSPhotoLibraryUsageDescription</key>");
    expect(infoPlist).toContain(
      "<string>OOMU reads only the photo details you ask it to review.</string>",
    );
    expect(readFileSync(join(root, "src-tauri", "build.rs"), "utf8")).toContain(
      '"read_system_photos"',
    );
    for (const capability of ["default", "development", "production"]) {
      const manifest = JSON.parse(
        readFileSync(join(root, "src-tauri", "capabilities", `${capability}.json`), "utf8"),
      );
      expect(manifest.permissions).toContain("allow-read-system-photos");
    }
  });

  it("keeps the durable recovery-state projection reachable in every desktop build", () => {
    const command = "get_agent_execution_recovery_states";
    const allowPermission = "allow-get-agent-execution-recovery-states";
    const nativeRegistry = readFileSync(
      join(root, "src-tauri", "src", "command_registration.rs"),
      "utf8",
    );
    expect(nativeRegistry).toContain(`db::${command}`);
    const nativeShell = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
    expect(nativeShell).toContain("command_registration::oomu_command_handler!()");

    const buildManifest = readFileSync(join(root, "src-tauri", "build.rs"), "utf8");
    expect(buildManifest).toContain(`"${command}"`);

    const generatedPermission = readFileSync(
      join(root, "src-tauri", "permissions", "autogenerated", `${command}.toml`),
      "utf8",
    );
    expect(generatedPermission).toContain(`identifier = "${allowPermission}"`);
    expect(generatedPermission).toContain(`commands.allow = ["${command}"]`);

    for (const capability of ["default", "development", "production"]) {
      const manifest = JSON.parse(
        readFileSync(join(root, "src-tauri", "capabilities", `${capability}.json`), "utf8"),
      );
      expect(manifest.permissions).toContain(allowPermission);
    }
  });

  it("declares just-in-time Contacts access on the main OOMU bundle", () => {
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toContain("<key>NSContactsUsageDescription</key>");
    expect(infoPlist).toContain(
      "<string>OOMU reads only the contacts you ask it to find.</string>",
    );
    expect(verifyEntitlementSnapshot().entitlements).toMatchObject({
      "com.apple.security.personal-information.addressbook": true,
    });
  });

  it("declares just-in-time Media & Apple Music access on the main OOMU bundle", () => {
    const infoPlist = readFileSync(join(root, "src-tauri", "Info.plist"), "utf8");
    expect(infoPlist).toContain("<key>NSAppleMusicUsageDescription</key>");
    expect(infoPlist).toContain(
      "<string>OOMU reads only the music library metadata you ask it to review.</string>",
    );
    expect(readFileSync(join(root, "src-tauri", "build.rs"), "utf8")).toContain(
      '"read_system_music"',
    );
    for (const capability of ["default", "development", "production"]) {
      const manifest = JSON.parse(
        readFileSync(join(root, "src-tauri", "capabilities", `${capability}.json`), "utf8"),
      );
      expect(manifest.permissions).toContain("allow-read-system-music");
    }
  });

  it("keeps the native speech controller alive after macOS permission approval", () => {
    const bridgeSource = readFileSync(
      join(root, "src-tauri", "src-swift", "main.swift"),
      "utf8",
    );
    expect(bridgeSource).toContain("private let speechBridge = SpeechBridge()");
    expect(bridgeSource).toContain("speechBridge.run()");
    expect(bridgeSource).not.toContain("SpeechBridge().run()");
  });

});

describe("production release build controls", () => {

  it("creates exclusive build output directories from a clean release root", () => {
    const cleanRoot = temporaryRoot("clean-release-output");
    const evidence = join(cleanRoot, "release", "evidence", "build-214-output");
    const candidate = join(cleanRoot, "release", "candidates", "build-214-output");

    expect(existsSync(join(cleanRoot, "release", "evidence"))).toBe(false);
    expect(() => createExclusiveReleaseOutputDirectories(evidence, candidate)).not.toThrow();
    expect(existsSync(evidence)).toBe(true);
    expect(existsSync(candidate)).toBe(true);
    expect(() => createExclusiveReleaseOutputDirectories(evidence, candidate)).toThrow();
  });

  it("contains no Apple-tool shadows or PATH injection", () => {
    const packageJson = readFileSync(join(root, "package.json"), "utf8");
    expect(packageJson).not.toContain("scripts/bin");
    const parsedPackage = JSON.parse(packageJson);
    expect(parsedPackage.scripts["build:prod"]).toBe("node scripts/release.mjs");
    expect(parsedPackage.scripts["build:prod"]).not.toMatch(/source|sign_env\.sh|zsh|bash/);
    expect(readFileSync(join(root, "scripts", "preflight_signing.js"), "utf8")).not.toContain(
      'path.join(__dirname, "sign_env.sh")',
    );
    expect(existsSync(join(root, "scripts", "bin", "actool"))).toBe(false);
    expect(existsSync(join(root, "scripts", "bin", "sw_vers"))).toBe(false);
  });

  it("makes strict lint and source size unavoidable release gates", () => {
    const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    expect(packageJson.scripts.lint).toBe("eslint --max-warnings=0");
    expect(packageJson.scripts["check:source-size"]).toBe(
      "scripts/check-rust-file-lines.sh",
    );
    expect(packageJson.scripts["rust:line-report"]).toBe(
      "npm run check:source-size",
    );
    expect(packageJson.scripts["check:quality"]).toContain("npm run lint");
    expect(packageJson.scripts["check:quality"]).toContain("npm run check:source-size");

    const lineGate = readFileSync(
      join(root, "scripts", "check-rust-file-lines.sh"),
      "utf8",
    );
    expect(lineGate).toContain("readonly new_file_limit=1500");
    expect(lineGate).not.toContain('new_file_limit="${OOMU_NEW_SOURCE_LINE_LIMIT');
    expect(lineGate).toContain("HEADROOM");

    const verificationMatrix = readFileSync(
      join(root, "scripts", "verification-matrix.mjs"),
      "utf8",
    );
    expect(verificationMatrix).toContain(
      '["source-line-ratchet", "npm", ["run", "check:source-size"]]',
    );
    expect(verificationMatrix).toContain('["lint", "npm", ["run", "lint"]]');

    const release = readFileSync(join(root, "scripts", "release.mjs"), "utf8");
    expect(release).toContain(
      'runStep("automated_strict_lint", npm, ["run", "lint"]',
    );
    expect(release).toContain(
      'runStep("automated_source_size", npm, ["run", "check:source-size"]',
    );
    const canonicalEvidence = readFileSync(
      join(root, "scripts", "release-canonical-evidence.mjs"),
      "utf8",
    );
    const automatedEvidence = canonicalEvidence.slice(
      canonicalEvidence.indexOf('"automated_tests"'),
      canonicalEvidence.indexOf('"release_extension_gates"'),
    );
    expect(automatedEvidence).toContain("combinedExecution([");
    expect(automatedEvidence).toContain('"automated_strict_lint"');
    expect(automatedEvidence).toContain('"automated_source_size"');
    expect(canonicalEvidence).toContain("source_line_baseline_sha256");
    expect(canonicalEvidence).toContain("lint_warnings: 0");
    expect(canonicalEvidence).toContain("source_size_violations: 0");
  });

  it("rejects ignored dotenv inputs and ambient release-affecting overrides", () => {
    const cleanCheckout = temporaryRoot("release-inputs");
    expect(() => assertNoRepositoryDotenvFiles(cleanCheckout)).not.toThrow();

    writeFileSync(join(cleanCheckout, ".env.production"), "NEXT_PUBLIC_OOMU_DEVELOPER_BUILD=true\n");
    expect(() => assertNoRepositoryDotenvFiles(cleanCheckout)).toThrow(
      /\.env\.production/i,
    );

    for (const environment of [
      { NEXT_PUBLIC_OOMU_DEVELOPER_BUILD: "true" },
      { NEXT_PUBLIC_UNREVIEWED_RELEASE_INPUT: "enabled" },
      { OOMU_PORTABLE_PYTHON_SHA256: "0".repeat(64) },
      { TAURI_ENV_TARGET_TRIPLE: "x86_64-apple-darwin" },
      { CARGO_BUILD_TARGET: "x86_64-apple-darwin" },
      { NODE_OPTIONS: "--require=/tmp/unreviewed.cjs" },
    ]) {
      expect(() => assertNoReleaseEnvironmentOverrides(environment)).toThrow(
        /environment overrides are prohibited/i,
      );
      expect(() => sanitizedChildEnvironment({}, environment)).toThrow(
        /environment overrides are prohibited/i,
      );
    }
    expect(() =>
      assertNoReleaseEnvironmentOverrides({ PATH: "/usr/bin", HOME: "/tmp/home" }),
    ).not.toThrow();
  });

  it("requires exactly the advertised ARM64 release architecture", () => {
    expect(assertExpectedReleaseArchitecture("arm64\n")).toBe("arm64");
    for (const reported of ["", "x86_64", "arm64 x86_64", "x86_64 arm64"]) {
      expect(() => assertExpectedReleaseArchitecture(reported)).toThrow(
        /must be exactly arm64/i,
      );
    }
  });

});

describe("production release security boundaries", () => {
  it("guards package, direct Tauri bundle, direct release builds, and Linux packaging", () => {
    const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    expect(packageJson.scripts["tauri:build"]).toBeUndefined();
    expect(packageJson.scripts["tauri:build:internal"]).toContain("assert-release-entrypoint");
    expect(packageJson.scripts["tauri:bundle:internal"]).toContain("assert-release-entrypoint");
    expect(packageJson.scripts["sign:local-app"]).toBe(
      "node scripts/sign-local-macos-app.mjs",
    );
    expect(packageJson.scripts["fleet:build:mac"]).toBe("npm run build:prod");
    expect(packageJson.scripts["fleet:build:linux"]).toContain("release.mjs --platform linux");

    const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
    expect(tauri.build.beforeBundleCommand).toContain("assert-release-entrypoint");
    expect(tauri.bundle.externalBin).toContain("binaries/pdf_extract_helper");
    expect(tauri.bundle.externalBin).toContain("binaries/oomu-speech-bridge");
    expect(tauri.bundle.externalBin).not.toContain("binaries/oomu-eventkit-calendar-helper");
    expect(tauri.bundle.externalBin).not.toContain("binaries/whatsapp_sidecar");
    const releaseTauri = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.release.conf.json"), "utf8"),
    );
    expect(releaseTauri.bundle.externalBin).not.toContain("binaries/whatsapp_sidecar");
    expect(releaseTauri.bundle.externalBin).toContain("binaries/pdf_extract_helper");
    expect(releaseTauri.bundle.externalBin).toContain("binaries/oomu-speech-bridge");
    expect(releaseTauri.bundle.externalBin).not.toContain("binaries/oomu-eventkit-calendar-helper");
    const releaseEntrypoint = readFileSync(join(root, "scripts", "release.mjs"), "utf8");
    expect(releaseEntrypoint.match(/tauri\.release\.conf\.json/g)).toHaveLength(2);
    expectLocalSigningBoundary();
    const buildRs = readFileSync(join(root, "src-tauri", "build.rs"), "utf8");
    expect(buildRs).toContain("verify_release_entrypoint");
    expectContainsEach(buildRs, ["canonical-v1", "unsigned-v2", "OOMU_RELEASE_POLICY_SHA256",
      "OOMU_RELEASE_AUTHORIZATION_BASE64", TRUSTED_RELEASE_PUBLIC_KEY_HEX]);
    expect(readFileSync(join(root, "src-tauri", "src", "audit.rs"), "utf8")).toContain(
      TRUSTED_RELEASE_PUBLIC_KEY_HEX,
    );

    const unauthorizedEnvironment = withoutEnvironmentKeys(process.env, RELEASE_AUTHORIZATION_ENV_NAMES);
    const guard = spawnSync(process.execPath, ["scripts/assert-release-entrypoint.mjs"], {
      cwd: root,
      encoding: "utf8",
      env: unauthorizedEnvironment,
    });
    expect(guard.status).toBe(1);
    expect(guard.stderr).toContain("internal");

    const approvedLocalUnsignedGuard = spawnSync(
      process.execPath,
      ["scripts/assert-release-entrypoint.mjs"],
      {
        cwd: root,
        encoding: "utf8",
        env: { ...unauthorizedEnvironment, OOMU_LOCAL_UNSIGNED_BUILD: "1" },
      },
    );
    expect(approvedLocalUnsignedGuard.status).toBe(0);

    const directBundle = spawnSync(
      join(root, "node_modules", ".bin", "tauri"),
      ["bundle", "--config", "src-tauri/tauri.release.conf.json", "--bundles", "app", "--no-sign"],
      { cwd: root, encoding: "utf8", env: unauthorizedEnvironment },
    );
    expect(directBundle.status).toBe(1);
    expect(`${directBundle.stdout}${directBundle.stderr}`).toContain("distributable Tauri steps are internal");

    const linux = spawnSync("/bin/bash", ["tools/build_linux_deb.sh"], {
      cwd: root,
      encoding: "utf8",
    });
    expect(linux.status).toBe(1);
    expect(linux.stderr).toContain("disabled");
  });

  it("rejects a repository-local executable during Apple-tool resolution", () => {
    const fakeRepo = temporaryRoot("tool-shadow");
    const fakeBin = join(fakeRepo, "scripts", "bin");
    mkdirSync(fakeBin, { recursive: true });
    const fakeActool = join(fakeBin, "actool");
    writeFileSync(fakeActool, "#!/bin/sh\nexit 0\n");
    chmodSync(fakeActool, 0o755);
    expect(() => assertTrustedAppleToolPath("actool", fakeActool, fakeRepo)).toThrow(
      /repository-local|scripts\/bin/i,
    );

    const cleanCheckout = temporaryRoot("tool-no-shadow-directory");
    expect(existsSync(join(cleanCheckout, "scripts", "bin"))).toBe(false);
    expect(assertTrustedAppleToolPath("true", "/usr/bin/true", cleanCheckout)).toBe(
      resolve("/usr/bin/true"),
    );
  });

  it("rejects tool resolution outside the active developer directory", () => {
    const fakeDeveloper = temporaryRoot("fake-developer-dir");
    const externalTool = "/usr/bin/true";
    expect(() =>
      assertAppleDeveloperToolPath("xcodebuild", externalTool, fakeDeveloper),
    ).toThrow(/outside the active Xcode developer directory/i);

    const result = spawnSync(
      process.execPath,
      ["scripts/preflight-apple-toolchain.mjs", "--output", join(fakeDeveloper, "report.json")],
      {
        cwd: root,
        encoding: "utf8",
        env: { ...process.env, DEVELOPER_DIR: fakeDeveloper },
      },
    );
    expect(result.status).not.toBe(0);
    expect(result.stderr).toMatch(/developer|xcrun|xcode/i);
  });

  it("uses repository-owned runner entry points backed only by the real mTLS release lab", () => {
    const release = readFileSync(join(root, "scripts", "release.mjs"), "utf8");
    const client = readFileSync(
      join(root, "scripts", "release-runners", "remote-runner-client.mjs"),
      "utf8",
    );
    for (const runner of ["clean-machine-launch.mjs", "p0-acceptance.mjs"]) {
      expect(release).toContain(`repositoryReleaseRunner(\"${runner}\")`);
    }
    expect(release).not.toContain("OOMU_CLEAN_MACHINE_TEST_RUNNER");
    expect(client).toContain('minVersion: "TLSv1.3"');
    expect(client).toContain("rejectUnauthorized: true");
    expect(client).not.toMatch(/synthetic:\s*true|status:\s*[\"']passed[\"']/);
  });
});

describe("deterministic native release qualification", () => {
  it.each([
    ["release.mjs", 'runStep("automated_cargo_test"'],
    ["release-unsigned.mjs", '["cargo-test", ['],
  ])("serializes the full native suite in %s", (file, marker) => {
    const release = readFileSync(join(root, "scripts", file), "utf8");
    const cargoTestStart = release.indexOf(marker);
    const cargoTestEnd = release.indexOf("],", cargoTestStart);
    const cargoTest = release.slice(cargoTestStart, cargoTestEnd);

    expect(cargoTestStart).toBeGreaterThanOrEqual(0);
    expect(cargoTest).toContain('"test", "--locked", "--target"');
    expect(cargoTest).toContain('"--", "--test-threads=1"');
  });
});

describe("canonical release orchestration", () => {
  it("uses one Tauri preparation hook and the immutable executable toolchain", () => {
    const release = readFileSync(join(root, "scripts", "release.mjs"), "utf8");
    const tauri = JSON.parse(
      readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(release).not.toContain('runStep("native_release_preparation"');
    expect(tauri.build.beforeBuildCommand).toBe("npm run tauri:prepare");
    expect(release).toContain(
      "const built = buildAndSignApplication(context, immutableReleaseToolchain)",
    );
    expect(release).toContain(
      "const notarized = notarizeAndCreateDmg(context, immutableReleaseToolchain, built)",
    );
    expect(release).toContain(
      "context, immutableReleaseToolchain, built, notarized",
    );
  });
});

describe("production release credential boundaries", () => {
  it("does not expose release credentials or capabilities to child processes and harnesses", () => {
    const previous = { ...process.env };
    try {
      process.env.APPLE_CERTIFICATE_PASSWORD = "certificate-secret";
      process.env.APPLE_API_ISSUER = "issuer-secret";
      process.env.APPLE_API_KEY = "key-secret";
      process.env.APPLE_NOTARY_KEYCHAIN_PROFILE = "OOMU-notary";
      process.env.OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH = "/secret/release.pem";
      process.env.OOMU_RELEASE_AUTHORIZATION_BASE64 = "authorization-secret";
      process.env.OOMU_CLEAN_MACHINE_TEST_RUNNER = "/external/clean-runner";
      process.env.OOMU_UNRELATED_VALUE = "retained";

      const releaseSource = withoutEnvironmentKeys(process.env, [
        "NODE_ENV", "MACOSX_DEPLOYMENT_TARGET", "CARGO_ENCODED_RUSTFLAGS",
      ]);
      const childEnvironment = sanitizedChildEnvironment({}, releaseSource);
      expect(childEnvironment.OOMU_UNRELATED_VALUE).toBeUndefined();
      expect(childEnvironment.PATH).toBe(process.env.PATH);
      for (const name of [
        "APPLE_CERTIFICATE_PASSWORD",
        "APPLE_API_ISSUER",
        "APPLE_API_KEY",
        "APPLE_NOTARY_KEYCHAIN_PROFILE",
        "OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH",
        "OOMU_RELEASE_AUTHORIZATION_BASE64",
        "OOMU_CLEAN_MACHINE_TEST_RUNNER",
      ]) {
        expect(childEnvironment[name]).toBeUndefined();
      }

      const signingEnvironment = sanitizedChildEnvironment(
        { APPLE_NOTARY_KEYCHAIN_PROFILE: "OOMU-notary" },
        releaseSource,
      );
      expect(signingEnvironment.APPLE_NOTARY_KEYCHAIN_PROFILE).toBe("OOMU-notary");

      const harnessEnvironment = externalHarnessEnvironment({});
      expect(harnessEnvironment.OOMU_UNRELATED_VALUE).toBeUndefined();
      expect(harnessEnvironment.APPLE_CERTIFICATE_PASSWORD).toBeUndefined();
      expect(harnessEnvironment.OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH).toBeUndefined();
    } finally {
      for (const name of Object.keys(process.env)) {
        if (!(name in previous)) delete process.env[name];
      }
      Object.assign(process.env, previous);
    }
  });

  it("keeps signing authority local and requires handoff verification first", () => {
    expect(existsSync(join(root, ".github", "workflows", "release.yml"))).toBe(false);
    const protectedSource = readFileSync(
      join(root, "scripts", "release-protected-sign.mjs"), "utf8",
    );
    expectContainsEach(protectedSource, [
      "verifyAndExpandUnsignedHandoff",
      "APPLE_SIGNING_IDENTITY",
      "OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH",
    ]);
    expect(protectedSource.indexOf("verifyAndExpandUnsignedHandoff"))
      .toBeLessThan(protectedSource.indexOf('required("APPLE_SIGNING_IDENTITY")'));
  });

  it("requires a trusted signature over the exact build and source authorization", () => {
    const temp = temporaryRoot("release-authorization");
    const privatePath = join(temp, "private.pem");
    const publicPath = join(temp, "public.pem");
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    writeFileSync(privatePath, privateKey.export({ format: "pem", type: "pkcs8" }));
    writeFileSync(publicPath, publicKey.export({ format: "pem", type: "spki" }));
    const trustedPublicKeyHex = Buffer.from(
      publicKey.export({ format: "jwk" }).x,
      "base64url",
    ).toString("hex");
    const buildId = "build-214-authorization";
    const sourceRevision = "a".repeat(40);
    const signatureBase64 = createReleaseAuthorization(
      privatePath,
      buildId,
      sourceRevision,
      trustedPublicKeyHex,
    );
    expect(
      verifyReleaseAuthorization({
        buildId,
        sourceRevision,
        signatureBase64,
        publicKeyPath: publicPath,
        trustedPublicKeyHex,
      }),
    ).toBe(true);
    expect(
      verifyReleaseAuthorization({
        buildId: `${buildId}-tampered`,
        sourceRevision,
        signatureBase64,
        publicKeyPath: publicPath,
        trustedPublicKeyHex,
      }),
    ).toBe(false);
  });
});

describe("least-privilege entitlement review", () => {
  it("accepts the reviewed application entitlement profile", () => {
    expect(verifyEntitlementSnapshot().entitlements).toEqual({
      "com.apple.security.automation.apple-events": true,
      "com.apple.security.network.client": true,
      "com.apple.security.personal-information.addressbook": true,
      "com.apple.security.personal-information.calendars": true,
    });
  });

  it("rejects non-boolean entitlement expansion that a boolean-only parser would miss", () => {
    const temp = temporaryRoot("entitlement-expansion");
    const plist = join(temp, "entitlements.plist");
    const snapshot = join(temp, "snapshot.json");
    writeFileSync(
      plist,
      `<?xml version="1.0" encoding="UTF-8"?>
      <plist version="1.0"><dict>
      <key>com.apple.security.network.client</key><true/>
      <key>com.apple.security.application-groups</key><array><string>group.unreviewed</string></array>
      </dict></plist>`,
    );
    writeFileSync(
      snapshot,
      JSON.stringify({
        schema_version: 1,
        kind: "oomu.reviewed-entitlements",
        reviewed_entitlements: { "com.apple.security.network.client": true },
        sha256: "0".repeat(64),
      }),
    );
    expect(() =>
      verifyEntitlementSnapshot({ entitlementsPath: plist, snapshotPath: snapshot }),
    ).toThrow(/differ/i);
  });

  it("extracts and exactly compares entitlements from the final signed artifacts", () => {
    const temp = temporaryRoot("signed-entitlements");
    const application = join(temp, "oomu-app");
    copyFileSync("/bin/echo", application);
    chmodSync(application, 0o755);
    const signing = spawnSync(
      "/usr/bin/codesign",
      ["--force", "--sign", "-", "--entitlements", join(root, "src-tauri", "entitlements.plist"), application],
      { encoding: "utf8" },
    );
    expect(signing.status, signing.stderr).toBe(0);

    const snapshot = verifyFinalSignedEntitlements({ appPath: application });
    expect(snapshot.application.extracted.entitlements).toEqual({
      "com.apple.security.automation.apple-events": true,
      "com.apple.security.network.client": true,
      "com.apple.security.personal-information.addressbook": true,
      "com.apple.security.personal-information.calendars": true,
    });
    expect(() =>
      verifySignedEntitlements({
        artifactPath: application,
        expectedEntitlements: { "com.apple.security.network.client": true },
        label: "Application",
      }),
    ).toThrow(/differ from the reviewed exact set/i);
  });
});

describe("executed sanitizers", () => {
  it("rejects sanitizer-rule and database-looking symbolic links", () => {
    const general = temporaryRoot("sanitize-symlink");
    writeFileSync(join(general, "safe"), "safe");
    symlinkSync(join(general, "safe"), join(general, ".env"));
    const generalResult = spawnSync(
      process.execPath,
      ["scripts/sanitize-release.mjs", "--dir", general, "--execute"],
      { cwd: root, encoding: "utf8" },
    );
    expect(generalResult.status).toBe(1);
    expect(generalResult.stderr).toContain("symbolic link");

    const database = temporaryRoot("database-symlink");
    writeFileSync(join(database, "safe"), "safe");
    symlinkSync(join(database, "safe"), join(database, "payload.sqlite"));
    const databaseResult = spawnSync(
      process.execPath,
      ["scripts/sanitize-release-db.mjs", "--dir", database],
      { cwd: root, encoding: "utf8" },
    );
    expect(databaseResult.status).toBe(1);
    expect(databaseResult.stderr).toContain("symbolic link");
  });

  it("attests an executed zero-database scan instead of silently skipping", () => {
    const database = temporaryRoot("database-zero");
    const evidence = join(temporaryRoot("database-evidence"), "db.json");
    writeFileSync(join(database, "ordinary-resource.json"), "{}");
    const result = spawnSync(
      process.execPath,
      [
        "scripts/sanitize-release-db.mjs",
        "--dir",
        database,
        "--build-id",
        "build-214-test",
        "--evidence",
        evidence,
      ],
      { cwd: root, encoding: "utf8" },
    );
    expect(result.status).toBe(0);
    const report = JSON.parse(readFileSync(evidence, "utf8"));
    expect(report.status).toBe("passed");
    expect(report.database_count).toBe(0);
    expect(report.files_scanned).toBe(1);
    expect(report.synthetic).toBe(false);
  });

  it("binary-scans packaged resources and native executables for removed fixtures", () => {
    const packagedApp = temporaryRoot("packaged-fixture-scan");
    const nativeBinary = join(packagedApp, "oomu-native");
    writeFileSync(
      nativeBinary,
      Buffer.concat([
        Buffer.from([0x7f, 0x45, 0x4c, 0x46, 0x00, 0xff]),
        Buffer.from("run_supplier_reconciliation_audit", "utf8"),
        Buffer.from([0x00, 0xfe]),
      ]),
    );
    const result = spawnSync(
      process.execPath,
      ["scripts/sanitize-release.mjs", "--dir", packagedApp, "--execute"],
      { cwd: root, encoding: "utf8" },
    );
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("supplier-command-signature");
  });
});
