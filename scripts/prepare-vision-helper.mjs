import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const executableSuffix = process.platform === "win32" ? ".exe" : "";

function output(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
  return result.stdout.trim();
}

const targetTriple = output("rustc", ["--print", "host-tuple"]);
if (!targetTriple) {
  throw new Error("rustc did not return a host target triple.");
}

const bundledBinary = join(
  root,
  "src-tauri",
  "binaries",
  `oomu-vision-helper-${targetTriple}${executableSuffix}`,
);
mkdirSync(dirname(bundledBinary), { recursive: true });
const moduleCacheDir = join(root, "src-tauri", "target", "swift-module-cache");
mkdirSync(moduleCacheDir, { recursive: true });

if (process.platform !== "darwin") {
  if (!existsSync(bundledBinary) && process.platform !== "win32") {
    writeFileSync(
      bundledBinary,
      [
        "#!/bin/sh",
        "echo '{\"backend\":\"apple-vision-unavailable\",\"warnings\":[\"Apple Vision OCR is only available in the macOS desktop build.\"]}'",
        "exit 1",
        "",
      ].join("\n"),
    );
    chmodSync(bundledBinary, 0o755);
  }
  console.log("[vision-helper] Compilation skipped (non-macOS platform).");
  process.exit(0);
}

const swiftSource = join(root, "src-tauri", "src", "tools", "vision.swift");
if (!existsSync(swiftSource)) {
  throw new Error(`Swift vision helper source is missing at ${swiftSource}`);
}

const swiftArgs = [
  "-O",
  "-target",
  "arm64-apple-macos14.0",
  "-module-cache-path",
  moduleCacheDir,
  swiftSource,
  "-o",
  bundledBinary,
];

console.log("[vision-helper] Compiling Swift companion...");
const result = spawnSync("swiftc", swiftArgs, {
  cwd: root,
  stdio: "inherit",
  env: {
    ...process.env,
    MACOSX_DEPLOYMENT_TARGET: "14.0",
    CLANG_MODULE_CACHE_PATH: moduleCacheDir,
    MODULE_CACHE_DIR: moduleCacheDir,
  },
});
if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

chmodSync(bundledBinary, 0o755);
console.log(`[vision-helper] Compiled native OCR sidecar helper for ${targetTriple}.`);
