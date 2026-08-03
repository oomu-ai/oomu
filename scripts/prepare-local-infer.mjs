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

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

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
  `local_infer-${targetTriple}${executableSuffix}`,
);
mkdirSync(dirname(bundledBinary), { recursive: true });
if (!existsSync(bundledBinary)) {
  writeFileSync(bundledBinary, "");
}

const cargoArgs = [
  "build",
  "--manifest-path",
  manifestPath,
  "--bin",
  "local_infer",
];
if (release) {
  cargoArgs.push("--release");
}
run("cargo", cargoArgs);

const builtBinary = join(
  root,
  "src-tauri",
  "target",
  profile,
  `local_infer${executableSuffix}`,
);
copyFileSync(builtBinary, bundledBinary);
if (process.platform !== "win32") {
  chmodSync(bundledBinary, 0o755);
}

console.log(`[local_infer] Prepared ${profile} worker for ${targetTriple}.`);
