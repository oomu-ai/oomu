#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve, sep } from "node:path";
import process from "node:process";

const repositoryRoot = resolve(import.meta.dirname, "..");
export const releasePolicyPath = join(repositoryRoot, "release", "release-policy.json");

export function sha256Bytes(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) =>
      `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

export function loadReleasePolicy(path = releasePolicyPath) {
  const bytes = readFileSync(resolve(path));
  const policy = JSON.parse(bytes.toString("utf8"));
  if (
    policy.schemaVersion !== 1 ||
    !policy.policyId ||
    !/^\d+\.\d+\.\d+$/u.test(policy.node?.version ?? "") ||
    !/^\d+\.\d+\.\d+$/u.test(policy.node?.npmVersion ?? "") ||
    !/^\d+\.\d+\.\d+$/u.test(policy.rust?.channel ?? "") ||
    !/^\d+\.\d+\.\d+$/u.test(policy.rust?.cargoAuditVersion ?? "") ||
    policy.rust?.target !== policy.target ||
    !policy.protectedRunner?.label ||
    policy.protectedRunner?.architecture !== "arm64" ||
    !/^macos\d+$/u.test(policy.protectedRunner?.imageOs ?? "") ||
    !/^\d{8}\.\d+\.\d+$/u.test(policy.protectedRunner?.imageVersion ?? "") ||
    !isAbsolute(policy.protectedRunner?.xcode?.developerDirectory ?? "") ||
    !isAbsolute(policy.localRunner?.xcode?.developerDirectory ?? "")
  ) {
    throw new Error("The committed release policy is incomplete or invalid.");
  }
  const requiredActions = ["checkout", "setupNode", "uploadArtifact", "downloadArtifact"];
  if (requiredActions.some((name) => !(name in (policy.actions ?? {})))) {
    throw new Error("The committed release policy is missing a required Action pin.");
  }
  for (const [name, sha] of Object.entries(policy.actions ?? {})) {
    if (!/^[0-9a-f]{40}$/u.test(sha)) {
      throw new Error(`Release Action ${name} is not pinned to a full commit SHA.`);
    }
  }
  return { policy, policyDigest: sha256Bytes(bytes), policyPath: realpathSync(resolve(path)) };
}

function commandOutput(executable, args, environment) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    env: environment,
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${executable} identity probe failed.`);
  }
  return (result.stdout || result.stderr || "").trim();
}

function exactExecutable(label, expectedPath) {
  if (!isAbsolute(expectedPath) || !existsSync(expectedPath)) {
    throw new Error(`${label} is unavailable at its approved absolute location.`);
  }
  const executable = realpathSync(expectedPath);
  const metadata = statSync(executable);
  if (!metadata.isFile()) throw new Error(`${label} is not a regular executable file.`);
  accessSync(executable, constants.X_OK);
  return {
    label,
    executable,
    sha256: sha256Bytes(readFileSync(executable)),
    sizeBytes: metadata.size,
  };
}

function assertInside(label, child, parent) {
  const between = child.slice(parent.length);
  if (child !== parent && (!child.startsWith(`${parent}${sep}`) || between.includes(`..${sep}`))) {
    throw new Error(`${label} resolved outside its approved installation.`);
  }
}

function hostRustToolchainDirectory(channel) {
  const host = process.arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  return join(homedir(), ".rustup", "toolchains", `${channel}-${host}`, "bin");
}

function homebrewCmake() {
  return exactExecutable("CMake", "/opt/homebrew/bin/cmake");
}

function runnerProfile(policy, ciPhase) {
  return ciPhase ? policy.protectedRunner : policy.localRunner;
}

function assertRunnerIdentity(runner, ciPhase) {
  if (process.arch !== runner.architecture) {
    throw new Error(
      `Release runner architecture drifted: expected ${runner.architecture}, found ${process.arch}.`,
    );
  }
  if (!ciPhase) return;
  if (process.env.RUNNER_OS !== "macOS" || process.env.RUNNER_ARCH !== "ARM64") {
    throw new Error("Protected release runner identity does not match the reviewed macOS ARM64 runner.");
  }
  if (process.env.ImageOS !== runner.imageOs || process.env.ImageVersion !== runner.imageVersion) {
    throw new Error("Protected release runner image drifted from the reviewed image.");
  }
}

export function prioritizedExecutablePath(tools, preferredDirectory) {
  const directories = [preferredDirectory, ...Object.values(tools).map(({ executable }) => dirname(executable))];
  return [...new Set(directories)].join(":");
}

export function approvedReleaseEnvironment(toolchain, additions = {}) {
  const permitted = [
    "HOME", "TMPDIR", "TMP", "TEMP", "USER", "LOGNAME", "LANG", "LC_ALL",
    "CI", "GITHUB_ACTIONS", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion",
  ];
  const environment = Object.fromEntries(
    permitted.filter((name) => process.env[name] !== undefined)
      .map((name) => [name, process.env[name]]),
  );
  return {
    ...environment,
    PATH: toolchain.minimalPath,
    DEVELOPER_DIR: toolchain.runner.xcode.developerDirectory,
    MACOSX_DEPLOYMENT_TARGET: toolchain.policy.deploymentTarget,
    ...additions,
  };
}

export function collectReleaseToolchain({
  protectedPhase = false,
  ciPhase = process.env.GITHUB_ACTIONS === "true",
} = {}) {
  const { policy, policyDigest, policyPath } = loadReleasePolicy();
  const runner = runnerProfile(policy, ciPhase);
  assertRunnerIdentity(runner, ciPhase);

  const node = exactExecutable("Node.js", process.execPath);
  if (process.version !== `v${policy.node.version}`) {
    throw new Error(`Node.js drifted: expected ${policy.node.version}, found ${process.version}.`);
  }
  const nodeInstallation = dirname(node.executable);
  const npmCandidate = join(nodeInstallation, "npm");
  const npm = exactExecutable("npm", npmCandidate);
  assertInside("npm", npm.executable, resolve(nodeInstallation, ".."));

  const rustBin = hostRustToolchainDirectory(policy.rust.channel);
  const rustup = exactExecutable("rustup", join(homedir(), ".cargo", "bin", "rustup"));
  const cargo = exactExecutable("Cargo", join(rustBin, "cargo"));
  const rustc = exactExecutable("rustc", join(rustBin, "rustc"));
  const git = exactExecutable("Git", "/usr/bin/git");
  const xcodebuild = exactExecutable(
    "Xcode",
    join(runner.xcode.developerDirectory, "usr", "bin", "xcodebuild"),
  );
  const actool = exactExecutable(
    "actool",
    join(runner.xcode.developerDirectory, "usr", "bin", "actool"),
  );
  const clang = exactExecutable(
    "clang",
    join(
      runner.xcode.developerDirectory,
      "Toolchains", "XcodeDefault.xctoolchain", "usr", "bin", "clang",
    ),
  );
  const swiftc = exactExecutable(
    "swiftc",
    join(
      runner.xcode.developerDirectory,
      "Toolchains", "XcodeDefault.xctoolchain", "usr", "bin", "swiftc",
    ),
  );
  const notarytool = exactExecutable(
    "notarytool",
    join(runner.xcode.developerDirectory, "usr", "bin", "notarytool"),
  );
  const stapler = exactExecutable(
    "stapler",
    join(runner.xcode.developerDirectory, "usr", "bin", "stapler"),
  );
  const xcrun = exactExecutable("xcrun", "/usr/bin/xcrun");
  const xcodeSelect = exactExecutable("xcode-select", "/usr/bin/xcode-select");
  const codesign = exactExecutable("codesign", "/usr/bin/codesign");
  const xattr = exactExecutable("xattr", "/usr/bin/xattr");
  const ditto = exactExecutable("ditto", "/usr/bin/ditto");
  const hdiutil = exactExecutable("hdiutil", "/usr/bin/hdiutil");
  const security = exactExecutable("security", "/usr/bin/security");
  const file = exactExecutable("file", "/usr/bin/file");
  const plutil = exactExecutable("plutil", "/usr/bin/plutil");
  const sh = exactExecutable("sh", "/bin/sh");
  const assetutil = exactExecutable("assetutil", "/usr/bin/assetutil");
  const spctl = exactExecutable("spctl", "/usr/sbin/spctl");
  const lipo = exactExecutable(
    "lipo",
    join(runner.xcode.developerDirectory, "Toolchains", "XcodeDefault.xctoolchain", "usr", "bin", "lipo"),
  );
  const tools = {
    node, npm, rustup, cargo, rustc, git, xcodebuild, actool, assetutil, clang, swiftc, cmake: homebrewCmake(),
    notarytool, stapler, xcrun, xcodeSelect, codesign, xattr, ditto, hdiutil, security,
    file, plutil, sh, spctl, lipo,
  };
  const minimalPath = prioritizedExecutablePath(tools, rustBin);
  const provisional = { policy, runner, minimalPath };
  const environment = approvedReleaseEnvironment(provisional);
  const versions = {
    node: commandOutput(node.executable, ["--version"], environment),
    npm: commandOutput(npm.executable, ["--version"], environment),
    rustup: commandOutput(rustup.executable, ["--version"], environment),
    cargo: commandOutput(cargo.executable, ["--version"], environment),
    rustc: commandOutput(rustc.executable, ["--version"], environment),
    git: commandOutput(git.executable, ["--version"], environment),
    cmake: commandOutput(tools.cmake.executable, ["--version"], environment).split("\n")[0],
    xcode: commandOutput(xcodebuild.executable, ["-version"], environment),
    sdk: commandOutput(xcrun.executable, ["--sdk", "macosx", "--show-sdk-version"], environment),
  };
  if (versions.npm !== policy.node.npmVersion) throw new Error("npm version drifted from release policy.");
  if (!versions.rustc.startsWith(`rustc ${policy.rust.channel} `)) {
    throw new Error("rustc version drifted from release policy.");
  }
  if (!versions.xcode.includes(runner.xcode.version) || !versions.xcode.includes(runner.xcode.buildVersion)) {
    throw new Error("Xcode identity drifted from release policy.");
  }
  if (versions.sdk !== runner.xcode.sdkVersion) throw new Error("macOS SDK drifted from release policy.");
  return {
    schemaVersion: 1,
    phase: protectedPhase ? "protected" : ciPhase ? "unsigned" : "local",
    policy,
    policyPath,
    policyDigest,
    runner,
    tools,
    versions,
    minimalPath,
    collectedAt: new Date().toISOString(),
  };
}

export function assertToolUnchanged(tool) {
  const current = exactExecutable(tool.label, tool.executable);
  if (current.executable !== tool.executable || current.sha256 !== tool.sha256) {
    throw new Error(`${tool.label} changed after release preflight.`);
  }
}

export function releaseFailureDiagnostic(stdout, stderr, limit = 64 * 1024) {
  const stderrBudget = Math.floor(limit * 0.75);
  const stdoutBudget = limit - stderrBudget;
  const stderrTail = `${stderr ?? ""}`.trim().slice(-stderrBudget);
  const stdoutTail = `${stdout ?? ""}`.trim().slice(-stdoutBudget);
  return [
    stderrTail ? `--- stderr ---\n${stderrTail}` : "",
    stdoutTail ? `--- stdout tail ---\n${stdoutTail}` : "",
  ].filter(Boolean).join("\n");
}

export function runApproved(toolchain, toolName, args, options = {}) {
  const tool = toolchain.tools[toolName];
  if (!tool) throw new Error(`Release tool ${toolName} is not approved.`);
  assertToolUnchanged(tool);
  const result = spawnSync(tool.executable, args, {
    cwd: options.cwd ?? repositoryRoot,
    env: approvedReleaseEnvironment(toolchain, options.environment),
    encoding: "utf8",
    maxBuffer: options.maxBuffer ?? 256 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (options.exposeFailureOutput === true) {
      const diagnostic = releaseFailureDiagnostic(result.stdout, result.stderr);
      if (diagnostic) {
        process.stderr.write(`${diagnostic}\n`);
      }
    }
    throw new Error(`${tool.label} failed in ${options.label ?? "release step"}.`);
  }
  return result;
}

function main() {
  const protectedPhase = process.argv.includes("--protected");
  const outputIndex = process.argv.indexOf("--output");
  const outputPath = outputIndex >= 0 ? process.argv[outputIndex + 1] : null;
  const evidence = collectReleaseToolchain({ protectedPhase });
  const serialized = `${JSON.stringify(evidence, null, 2)}\n`;
  if (outputPath) writeFileSync(resolve(outputPath), serialized, { mode: 0o600 });
  else process.stdout.write(serialized);
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(import.meta.filename)) {
  main();
}
