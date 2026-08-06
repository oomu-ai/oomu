#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import {
  closeSync,
  copyFileSync,
  cpSync,
  existsSync,
  chmodSync,
  mkdirSync,
  openSync,
  readFileSync,
  realpathSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const DEVELOPMENT_IDENTIFIER = "ai.eldris.oomu.gpd.development";
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const targetRoot = join(repositoryRoot, "src-tauri", "target");
const developmentAppPath = join(targetRoot, "development-bundle", "OOMU Development.app");
const developmentExecutablePath = join(developmentAppPath, "Contents", "MacOS", "oomu");
const developmentLaunchLockPath = join(targetRoot, ".oomu-tauri-dev-launch.lock");
const entitlementsPath = join(repositoryRoot, "src-tauri", "entitlements.plist");
const sourceInfoPlistPath = join(repositoryRoot, "src-tauri", "Info.plist");
const sourceLocalizationsPath = join(repositoryRoot, "src-tauri", "macos-localizations");
const tauriConfig = JSON.parse(
  readFileSync(join(repositoryRoot, "src-tauri", "tauri.conf.json"), "utf8"),
);

function fail(message) {
  process.stderr.write(`[tauri-dev-runner] ${message}\n`);
  process.exit(1);
}

function run(command, args, label) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    fail(`${label} failed${detail ? `:\n${detail}` : "."}`);
  }
  return result;
}

function setPlistValue(plistPath, key, type, value) {
  const setResult = spawnSync(
    "/usr/libexec/PlistBuddy",
    ["-c", `Set :${key} ${value}`, plistPath],
    { encoding: "utf8" },
  );
  if (setResult.status === 0) return;
  run(
    "/usr/libexec/PlistBuddy",
    ["-c", `Add :${key} ${type} ${value}`, plistPath],
    `Writing ${key} to the development app identity`,
  );
}

function validDevelopmentIdentity(value) {
  return /^(?:Apple Development|Developer ID Application): .+\([A-Z0-9]{10}\)$/u.test(value);
}

function developmentSigningIdentity() {
  const requested = process.env.OOMU_TAURI_DEV_SIGNING_IDENTITY?.trim();
  if (requested) {
    if (!validDevelopmentIdentity(requested)) {
      fail("OOMU_TAURI_DEV_SIGNING_IDENTITY is not an Apple code-signing identity.");
    }
    return requested;
  }
  const result = spawnSync(
    "/usr/bin/security",
    ["find-identity", "-v", "-p", "codesigning"],
    { encoding: "utf8" },
  );
  if (result.status !== 0) return "-";
  const identities = [...result.stdout.matchAll(/"([^"]+)"/gu)]
    .map((match) => match[1])
    .filter(validDevelopmentIdentity);
  const appleDevelopment = identities.filter((identity) => identity.startsWith("Apple Development:"));
  if (appleDevelopment.length === 1) return appleDevelopment[0];
  return identities.length === 1 ? identities[0] : "-";
}

function regexLiteral(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function exactDevelopmentPids() {
  const result = spawnSync(
    "/usr/bin/pgrep",
    ["-f", `^${regexLiteral(developmentExecutablePath)}([[:space:]]|$)`],
    { encoding: "utf8" },
  );
  if (result.status !== 0) return [];
  return result.stdout
    .trim()
    .split(/\s+/u)
    .map((value) => Number.parseInt(value, 10))
    .filter((pid) => Number.isSafeInteger(pid) && pid > 1);
}

function processExists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function acquireDevelopmentLaunchLock(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  const sleeper = new Int32Array(new SharedArrayBuffer(Int32Array.BYTES_PER_ELEMENT));
  while (Date.now() < deadline) {
    try {
      const descriptor = openSync(developmentLaunchLockPath, "wx", 0o600);
      writeFileSync(descriptor, `${process.pid}\n`, "utf8");
      return () => {
        closeSync(descriptor);
        try {
          unlinkSync(developmentLaunchLockPath);
        } catch (error) {
          if (error?.code !== "ENOENT") throw error;
        }
      };
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      let owner = Number.NaN;
      try {
        owner = Number.parseInt(readFileSync(developmentLaunchLockPath, "utf8").trim(), 10);
      } catch {
        // A writer may still be recording its PID. Wait before treating it as stale.
      }
      if (Number.isSafeInteger(owner) && owner > 1 && !processExists(owner)) {
        try {
          unlinkSync(developmentLaunchLockPath);
        } catch (unlinkError) {
          if (unlinkError?.code !== "ENOENT") throw unlinkError;
        }
        continue;
      }
      Atomics.wait(sleeper, 0, 0, 50);
    }
  }
  fail("Another OOMU development launch did not finish in time.");
}

function waitForDevelopmentExit(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  const sleeper = new Int32Array(new SharedArrayBuffer(Int32Array.BYTES_PER_ELEMENT));
  while (Date.now() < deadline) {
    if (exactDevelopmentPids().length === 0) return true;
    Atomics.wait(sleeper, 0, 0, 50);
  }
  return exactDevelopmentPids().length === 0;
}

function stopDevelopmentAppBeforeReplacement() {
  const existing = exactDevelopmentPids();
  if (existing.length === 0) return;
  for (const pid of existing) process.kill(pid, "SIGTERM");
  if (waitForDevelopmentExit(3_000)) return;
  for (const pid of exactDevelopmentPids()) process.kill(pid, "SIGKILL");
  if (!waitForDevelopmentExit(1_000)) {
    fail("The previous OOMU Development process did not exit before replacement.");
  }
}

const runnerArgs = process.argv.slice(2);
if (runnerArgs[0] !== "run") fail("Tauri did not request the reviewed Cargo run operation.");
const separatorIndex = runnerArgs.indexOf("--");
const cargoRunArgs = separatorIndex >= 0 ? runnerArgs.slice(0, separatorIndex) : runnerArgs;
const applicationArgs = separatorIndex >= 0 ? runnerArgs.slice(separatorIndex + 1) : [];
const diagnosticLaunch = applicationArgs.some((value) =>
  ["--audit-db", "--dump-db", "--help", "-h"].includes(value)
);
const cargoBuildArgs = ["build", ...cargoRunArgs.slice(1)];
run("cargo", cargoBuildArgs, "Building the OOMU development executable");

const targetArgumentIndex = cargoRunArgs.findIndex((value) => value === "--target");
const targetArgument = targetArgumentIndex >= 0
  ? cargoRunArgs[targetArgumentIndex + 1]
  : cargoRunArgs.find((value) => value.startsWith("--target="))?.slice("--target=".length);
const profile = cargoRunArgs.includes("--release") ? "release" : "debug";
const binaryPath = realpathSync(join(targetRoot, ...(targetArgument ? [targetArgument] : []), profile, "oomu"));
if (!binaryPath.startsWith(`${realpathSync(targetRoot)}${sep}`) || basename(binaryPath) !== "oomu") {
  fail("Refusing to launch an executable outside OOMU's Cargo target directory.");
}

let launchPath = binaryPath;
let launchBundlePath = null;
let releaseDevelopmentLaunchLock = null;
if (process.platform === "darwin") {
  if (!existsSync(entitlementsPath) || !existsSync(sourceInfoPlistPath)) {
    fail("The reviewed macOS permission metadata is missing.");
  }
  const appPath = developmentAppPath;
  const contentsPath = join(appPath, "Contents");
  const macosPath = join(contentsPath, "MacOS");
  const resourcesPath = join(contentsPath, "Resources");
  const bundledExecutablePath = join(macosPath, "oomu");
  const bundledInfoPlistPath = join(contentsPath, "Info.plist");
  const signingIdentity = developmentSigningIdentity();
  // Never replace a signed executable while macOS is still executing it. Doing so
  // invalidates resident code pages and causes a CODESIGNING/Invalid Page kill.
  releaseDevelopmentLaunchLock = acquireDevelopmentLaunchLock(120_000);
  stopDevelopmentAppBeforeReplacement();
  mkdirSync(macosPath, { recursive: true });
  mkdirSync(resourcesPath, { recursive: true });
  copyFileSync(binaryPath, bundledExecutablePath);
  chmodSync(bundledExecutablePath, 0o755);
  copyFileSync(sourceInfoPlistPath, bundledInfoPlistPath);
  cpSync(sourceLocalizationsPath, resourcesPath, { recursive: true });
  const targetTriple = process.arch === "arm64"
    ? "aarch64-apple-darwin"
    : "x86_64-apple-darwin";
  for (const configuredPath of tauriConfig.bundle.externalBin) {
    const sourcePath = join(repositoryRoot, "src-tauri", `${configuredPath}-${targetTriple}`);
    const destinationPath = join(macosPath, basename(configuredPath));
    if (!existsSync(sourcePath)) fail(`Configured development helper is missing: ${configuredPath}`);
    copyFileSync(sourcePath, destinationPath);
    chmodSync(destinationPath, 0o755);
    run(
      "/usr/bin/codesign",
      ["--force", "--timestamp=none", "--sign", signingIdentity, destinationPath],
      `Signing ${basename(configuredPath)}`,
    );
  }
  cpSync(
    join(repositoryRoot, "src-tauri", "resources"),
    join(resourcesPath, "resources"),
    { recursive: true },
  );
  mkdirSync(join(resourcesPath, "_up_", "src"), { recursive: true });
  cpSync(
    join(repositoryRoot, "src", "locales"),
    join(resourcesPath, "_up_", "src", "locales"),
    { recursive: true },
  );
  copyFileSync(
    join(repositoryRoot, "THIRD_PARTY_NOTICES.md"),
    join(resourcesPath, "_up_", "THIRD_PARTY_NOTICES.md"),
  );
  copyFileSync(
    join(repositoryRoot, "src-tauri", "icons", "icon.icns"),
    join(resourcesPath, "OOMU.icns"),
  );
  for (const [key, type, value] of [
    ["CFBundleDevelopmentRegion", "string", "en"],
    ["CFBundleDisplayName", "string", "OOMU Development"],
    ["CFBundleExecutable", "string", "oomu"],
    ["CFBundleIdentifier", "string", DEVELOPMENT_IDENTIFIER],
    ["CFBundleInfoDictionaryVersion", "string", "6.0"],
    ["CFBundleName", "string", "OOMU Development"],
    ["CFBundlePackageType", "string", "APPL"],
    ["CFBundleShortVersionString", "string", "0.1.3"],
    ["CFBundleVersion", "string", "8"],
  ]) {
    setPlistValue(bundledInfoPlistPath, key, type, value);
  }
  run(
    "/usr/bin/codesign",
    [
      "--force",
      "--timestamp=none",
      "--options",
      "runtime",
      "--sign",
      signingIdentity,
      "--entitlements",
      entitlementsPath,
      appPath,
    ],
    "Signing the isolated development app",
  );
  run(
    "/usr/bin/codesign",
    ["--verify", "--deep", "--strict", "--verbose=4", appPath],
    "Verifying the isolated development app",
  );
  const details = run(
    "/usr/bin/codesign",
    ["-d", "--verbose=4", appPath],
    "Reading the development signature",
  ).stderr;
  if (
    !details.includes(`Identifier=${DEVELOPMENT_IDENTIFIER}`) ||
    !details.includes("Info.plist entries=")
  ) {
    fail("The development executable did not retain its isolated identifier.");
  }
  launchPath = bundledExecutablePath;
  launchBundlePath = appPath;
  process.stdout.write(
    `[tauri-dev-runner] Native macOS app identity ready (${DEVELOPMENT_IDENTIFIER}; ${signingIdentity === "-" ? "ad hoc" : "Apple team"}).\n`,
  );
}

if (launchBundlePath) {
  if (exactDevelopmentPids().length > 0) {
    fail("An OOMU Development instance is already running.");
  }
}

const child = launchBundlePath && !diagnosticLaunch
  ? spawn(
      "/usr/bin/open",
      ["-n", "-W", launchBundlePath, ...(applicationArgs.length ? ["--args", ...applicationArgs] : [])],
      { env: process.env, stdio: "inherit" },
    )
  : spawn(launchPath, applicationArgs, {
  env: process.env,
  stdio: "inherit",
  });

if (launchBundlePath && !diagnosticLaunch) {
  const started = (() => {
    const deadline = Date.now() + 30_000;
    const sleeper = new Int32Array(new SharedArrayBuffer(Int32Array.BYTES_PER_ELEMENT));
    while (Date.now() < deadline) {
      if (exactDevelopmentPids().length > 0) return true;
      Atomics.wait(sleeper, 0, 0, 50);
    }
    return exactDevelopmentPids().length > 0;
  })();
  releaseDevelopmentLaunchLock?.();
  releaseDevelopmentLaunchLock = null;
  if (!started) fail("The signed OOMU development app did not start.");
} else if (launchBundlePath) {
  releaseDevelopmentLaunchLock?.();
  releaseDevelopmentLaunchLock = null;
}

function stopExactDevelopmentApp(signal) {
  if (!launchBundlePath) return;
  for (const pid of exactDevelopmentPids()) process.kill(pid, signal);
}

let forwardedSignal = null;
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => {
    forwardedSignal = signal;
    stopExactDevelopmentApp(signal);
    if (!child.killed) child.kill(signal);
  });
}

child.once("error", (error) => fail(`OOMU could not launch: ${error.message}`));
child.once("exit", (code, signal) => {
  if (signal && !forwardedSignal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? (signal ? 1 : 0));
});
