import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const suffix = process.platform === "win32" ? ".exe" : "";

function output(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8", env: process.env });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
  return result.stdout.trim();
}

const triple = output("rustc", ["--print", "host-tuple"]);
const bundled = join(
  root,
  "src-tauri",
  "binaries",
  `oomu-artifact-pdf-helper-${triple}${suffix}`,
);
mkdirSync(dirname(bundled), { recursive: true });

if (process.platform !== "darwin") {
  if (!existsSync(bundled) && process.platform !== "win32") {
    writeFileSync(
      bundled,
      "#!/bin/sh\necho '{\"backend\":\"oomu-artifact-pdf-helper/unavailable\",\"available\":false}'\nexit 1\n",
    );
    chmodSync(bundled, 0o755);
  }
  console.log("[artifact-pdf-helper] Compilation skipped (non-macOS platform).");
  process.exit(0);
}

const source = join(root, "src-tauri", "src", "tools", "artifact_pdf.swift");
if (!existsSync(source)) throw new Error(`Artifact PDF helper source is missing at ${source}`);
const moduleCache = join(root, "src-tauri", "target", "swift-module-cache");
mkdirSync(moduleCache, { recursive: true });
const result = spawnSync(
  "swiftc",
  [
    "-O",
    "-target",
    "arm64-apple-macos14.0",
    "-module-cache-path",
    moduleCache,
    source,
    "-o",
    bundled,
  ],
  {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      MACOSX_DEPLOYMENT_TARGET: "14.0",
      CLANG_MODULE_CACHE_PATH: moduleCache,
      MODULE_CACHE_DIR: moduleCache,
    },
  },
);
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
chmodSync(bundled, 0o755);
console.log(`[artifact-pdf-helper] Compiled native PDF builder/renderer for ${triple}.`);
