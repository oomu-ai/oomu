#!/usr/bin/env node

import { createHash } from "node:crypto";
import { accessSync, constants, existsSync, readFileSync, realpathSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { atomicWriteFile } from "./release-manifest.mjs";

const repoRoot = resolve(import.meta.dirname, "..");
const xcrunPath = "/usr/bin/xcrun";

function run(executable, args) {
  const result = spawnSync(executable, args, {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${executable} ${args.join(" ")} failed: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
}

function runProbe(executable, args) {
  const result = spawnSync(executable, args, {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) throw result.error;
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  return { exit_code: result.status, output };
}

function isInside(root, candidate) {
  const canonicalRoot = existsSync(root) ? realpathSync(resolve(root)) : resolve(root);
  const canonicalCandidate = existsSync(candidate)
    ? realpathSync(resolve(candidate))
    : resolve(candidate);
  const between = relative(canonicalRoot, canonicalCandidate);
  return between === "" || (!between.startsWith("..") && !between.startsWith(sep));
}

export function assertTrustedAppleToolPath(toolName, candidate, root = repoRoot) {
  if (!candidate?.startsWith("/")) {
    throw new Error(`${toolName} did not resolve to an absolute executable path.`);
  }
  const resolved = realpathSync(candidate);
  accessSync(resolved, constants.X_OK);
  if (isInside(root, resolved)) {
    throw new Error(`${toolName} resolved to a repository-local shadow executable: ${resolved}`);
  }
  const localShadowRoot = resolve(root, "scripts", "bin");
  if (isInside(localShadowRoot, resolved)) {
    throw new Error(`${toolName} resolved under scripts/bin: ${resolved}`);
  }
  return resolved;
}

function xcrunFind(toolName) {
  const resolved = run(xcrunPath, ["--find", toolName]).split(/\r?\n/).at(-1)?.trim();
  return assertTrustedAppleToolPath(toolName, resolved);
}

function appleCodeSignature(toolName, executable, requireXcodeTeam = false) {
  const details = runProbe("/usr/bin/codesign", ["-dvvv", executable]);
  if (details.exit_code !== 0) {
    throw new Error(`${toolName} code-signature metadata is unavailable.`);
  }
  const identifier = details.output.match(/^Identifier=(.+)$/m)?.[1]?.trim();
  const teamId = details.output.match(/^TeamIdentifier=(.+)$/m)?.[1]?.trim();
  const authority = details.output.match(/^Authority=(.+)$/m)?.[1]?.trim();
  const platformIdentifier = details.output.match(/^Platform identifier=(.+)$/m)?.[1]?.trim();
  if (!identifier?.startsWith("com.apple.")) {
    throw new Error(`${toolName} is not signed with an Apple tool identifier.`);
  }
  if (requireXcodeTeam && teamId !== "59GAB85EFG") {
    throw new Error(`${toolName} is not signed by the Apple Xcode tool team.`);
  }
  const requirements = runProbe("/usr/bin/codesign", ["-d", "-r-", executable]);
  if (requirements.exit_code !== 0 || !/\banchor apple\b/.test(requirements.output)) {
    throw new Error(`${toolName} does not carry an Apple-anchored designated requirement.`);
  }
  return {
    identifier,
    team_id: teamId ?? null,
    authority: authority ?? null,
    platform_identifier: platformIdentifier ?? null,
    designated_requirement: requirements.output
      .split(/\r?\n/)
      .find((line) => line.startsWith("designated =>")),
    signature_check: "apple-anchored-designated-requirement",
    verified_by: "/usr/bin/codesign",
  };
}

export function assertAppleDeveloperToolPath(toolName, candidate, developerDirectory) {
  const executable = assertTrustedAppleToolPath(toolName, candidate);
  const developer = realpathSync(resolve(developerDirectory));
  if (!isInside(developer, executable)) {
    throw new Error(`${toolName} resolved outside the active Xcode developer directory.`);
  }
  return executable;
}

function versionFor(toolName, executable) {
  const versionArgs = {
    xcodebuild: ["-version"],
    actool: ["--version"],
    assetutil: ["--version"],
    clang: ["--version"],
    swiftc: ["--version"],
    notarytool: ["--version"],
  }[toolName];
  if (versionArgs) {
    const probe = runProbe(executable, versionArgs);
    if (probe.exit_code === 0 && probe.output) {
      return {
        value: probe.output.split(/\r?\n/).slice(0, 3).join("\n"),
        source: `${executable} ${versionArgs.join(" ")}`,
      };
    }
  }
  const what = runProbe("/usr/bin/what", ["-q", executable]);
  const binarySha256 = createHash("sha256").update(readFileSync(executable)).digest("hex");
  return {
    value: what.output || `binary-sha256:${binarySha256}`,
    source: what.output ? `/usr/bin/what -q ${executable}` : "executable SHA-256 fallback",
    binary_sha256: binarySha256,
  };
}

export function inspectAppleToolchain() {
  if (process.platform !== "darwin") {
    throw new Error("A production OOMU release requires a clean macOS runner.");
  }
  assertTrustedAppleToolPath("xcrun", xcrunPath);
  const xcodeTools = [
    "xcodebuild",
    "actool",
    "assetutil",
    "clang",
    "swiftc",
    "codesign",
    "notarytool",
    "stapler",
    "lipo",
    "ditto",
    "hdiutil",
    "spctl",
  ];
  const tools = {};
  for (const toolName of xcodeTools) {
    const executable = xcrunFind(toolName);
    const version = versionFor(toolName, executable);
    tools[toolName] = {
      executable,
      version: version.value,
      version_source: version.source,
      binary_sha256: version.binary_sha256 ?? null,
      resolved_via: xcrunPath,
    };
  }
  const developerDirectory = realpathSync(
    resolve(dirname(tools.xcodebuild.executable), "..", ".."),
  );
  const selectedDeveloperDirectory = realpathSync(run("/usr/bin/xcode-select", ["-p"]));
  const configuredDeveloperDirectory = process.env.DEVELOPER_DIR
    ? realpathSync(resolve(process.env.DEVELOPER_DIR))
    : null;
  const expectedDeveloperDirectory = configuredDeveloperDirectory ?? selectedDeveloperDirectory;
  if (developerDirectory !== expectedDeveloperDirectory) {
    throw new Error(
      "xcrun tool resolution does not match the active DEVELOPER_DIR/xcode-select developer directory.",
    );
  }
  for (const toolName of [
    "xcodebuild",
    "actool",
    "clang",
    "swiftc",
    "notarytool",
    "stapler",
    "lipo",
  ]) {
    assertAppleDeveloperToolPath(toolName, tools[toolName].executable, developerDirectory);
    tools[toolName].code_signature = appleCodeSignature(
      toolName,
      tools[toolName].executable,
      true,
    );
  }
  const fixedSystemTools = {
    assetutil: "/usr/bin/assetutil",
    codesign: "/usr/bin/codesign",
    ditto: "/usr/bin/ditto",
    hdiutil: "/usr/bin/hdiutil",
    spctl: "/usr/sbin/spctl",
  };
  for (const [toolName, fixedPath] of Object.entries(fixedSystemTools)) {
    if (tools[toolName].executable !== realpathSync(fixedPath)) {
      throw new Error(`${toolName} did not resolve to the reviewed system executable.`);
    }
    tools[toolName].code_signature = appleCodeSignature(toolName, tools[toolName].executable);
  }
  const swVers = assertTrustedAppleToolPath("sw_vers", "/usr/bin/sw_vers");
  tools.sw_vers = {
    executable: swVers,
    version: run(swVers, ["-productVersion"]),
    resolved_via: "absolute_system_path",
    code_signature: appleCodeSignature("sw_vers", swVers),
  };
  const sdkPath = realpathSync(run(xcrunPath, ["--sdk", "macosx", "--show-sdk-path"]));
  if (!isInside(developerDirectory, sdkPath)) {
    throw new Error("The selected macOS SDK resolved outside the active Xcode developer directory.");
  }
  return {
    schema_version: 1,
    kind: "oomu.apple-toolchain-preflight",
    status: "passed",
    synthetic: false,
    inspected_at: new Date().toISOString(),
    developer_directory: developerDirectory,
    xcode_select_developer_directory: selectedDeveloperDirectory,
    developer_directory_environment: configuredDeveloperDirectory,
    sdk: {
      name: "macosx",
      path: sdkPath,
      version: run(xcrunPath, ["--sdk", "macosx", "--show-sdk-version"]),
    },
    tools,
  };
}

function parseOutputPath(argv) {
  if (argv.length === 0) return null;
  if (argv.length === 2 && argv[0] === "--output") return resolve(argv[1]);
  throw new Error("Usage: preflight-apple-toolchain.mjs [--output <path>]");
}

function main() {
  const outputPath = parseOutputPath(process.argv.slice(2));
  const report = inspectAppleToolchain();
  if (outputPath) atomicWriteFile(outputPath, `${JSON.stringify(report, null, 2)}\n`, 0o600);
  console.log(
    `Apple toolchain preflight passed: Xcode ${report.tools.xcodebuild.version.split("\n")[0]}, macOS SDK ${report.sdk.version}.`,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU APPLE TOOLCHAIN ERROR: ${error.message}`);
    process.exit(1);
  }
}
