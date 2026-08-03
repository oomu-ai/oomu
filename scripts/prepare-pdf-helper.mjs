import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const manifestPath = join(root, "src-tauri", "Cargo.toml");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const executableSuffix = process.platform === "win32" ? ".exe" : "";

function run(command, args, capture = false) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: capture ? "utf8" : undefined,
    stdio: capture ? "pipe" : "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (capture && result.stderr) process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
  return capture ? result.stdout.trim() : "";
}

const targetTriple = run("rustc", ["--print", "host-tuple"], true);
if (!targetTriple) throw new Error("rustc did not return a host target triple.");

const bundledBinary = join(
  root,
  "src-tauri",
  "binaries",
  `pdf_extract_helper-${targetTriple}${executableSuffix}`,
);
mkdirSync(dirname(bundledBinary), { recursive: true });
// Tauri validates every externalBin path from build.rs before Cargo can build
// that binary. This bootstrap file is always replaced by the successful build
// below and is never accepted as release evidence.
if (!existsSync(bundledBinary)) writeFileSync(bundledBinary, "");

const cargoArgs = [
  "build",
  "--manifest-path",
  manifestPath,
  "--bin",
  "pdf_extract_helper",
];
if (release) cargoArgs.push("--release");
run("cargo", cargoArgs);

const builtBinary = join(
  root,
  "src-tauri",
  "target",
  profile,
  `pdf_extract_helper${executableSuffix}`,
);
copyFileSync(builtBinary, bundledBinary);
if (process.platform !== "win32") chmodSync(bundledBinary, 0o755);

console.log(`[pdf-helper] Prepared contained ${profile} PDF parser for ${targetTriple}.`);
