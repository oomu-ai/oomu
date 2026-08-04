import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  statSync,
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

const helpers = [
  { name: "local_infer", bundledStem: "local_infer" },
  { name: "pdf_extract_helper", bundledStem: "pdf_extract_helper" },
  {
    name: "artifact_build_helper",
    bundledStem: "artifact_build_helper",
    adHocSignOnMacOs: true,
  },
].map((helper) => ({
  ...helper,
  builtPath: join(
    root,
    "src-tauri",
    "target",
    profile,
    `${helper.name}${executableSuffix}`,
  ),
  bundledPath: join(
    root,
    "src-tauri",
    "binaries",
    `${helper.bundledStem}-${targetTriple}${executableSuffix}`,
  ),
}));

// Tauri validates every externalBin path from build.rs before Cargo can build
// the corresponding target. Reserve all paths before the single Cargo pass;
// successful compilation replaces every reservation below.
for (const helper of helpers) {
  mkdirSync(dirname(helper.bundledPath), { recursive: true });
  if (!existsSync(helper.bundledPath)) writeFileSync(helper.bundledPath, "");
}

const cargoArgs = ["build", "--manifest-path", manifestPath];
for (const helper of helpers) cargoArgs.push("--bin", helper.name);
if (release) cargoArgs.push("--release");
run("cargo", cargoArgs);

// Confirm the complete Cargo batch exists before replacing any watched bundle
// input. This keeps one missing target from leaving a partially refreshed set.
for (const helper of helpers) {
  const metadata = statSync(helper.builtPath);
  if (!metadata.isFile() || metadata.size === 0) {
    throw new Error(`Cargo did not produce a substantive ${helper.name} binary.`);
  }
}

for (const helper of helpers) {
  copyFileSync(helper.builtPath, helper.bundledPath);
  if (process.platform !== "win32") chmodSync(helper.bundledPath, 0o755);
}

// The artifact builder is executed as a contained sidecar during development.
// Preserve its existing ad hoc signature so macOS does not terminate it before
// the startup probe; canonical release signing replaces this bundle signature.
if (process.platform === "darwin") {
  for (const helper of helpers.filter((candidate) => candidate.adHocSignOnMacOs)) {
    run("codesign", ["--force", "--sign", "-", helper.bundledPath]);
  }
}

console.log(
  `[rust-helpers] Prepared ${helpers.length} contained ${profile} helpers for ${targetTriple}.`,
);
