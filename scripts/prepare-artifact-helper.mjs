import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const manifest = join(root, "src-tauri", "Cargo.toml");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const suffix = process.platform === "win32" ? ".exe" : "";
function run(command, args, capture = false) {
  const result = spawnSync(command, args, { cwd: root, encoding: capture ? "utf8" : undefined, stdio: capture ? "pipe" : "inherit", env: process.env });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return capture ? result.stdout.trim() : "";
}
const triple = run("rustc", ["--print", "host-tuple"], true);
const bundled = join(root, "src-tauri", "binaries", `artifact_build_helper-${triple}${suffix}`);
mkdirSync(dirname(bundled), { recursive: true });
if (!existsSync(bundled)) writeFileSync(bundled, "");
const args = ["build", "--manifest-path", manifest, "--bin", "artifact_build_helper"];
if (release) args.push("--release");
run("cargo", args);
copyFileSync(join(root, "src-tauri", "target", profile, `artifact_build_helper${suffix}`), bundled);
if (process.platform !== "win32") chmodSync(bundled, 0o755);
// macOS validates the copied helper against its destination identity. Re-sign the
// copied sidecar so Gatekeeper does not terminate an otherwise byte-identical
// helper before its startup probe can run.
if (process.platform === "darwin") run("codesign", ["--force", "--sign", "-", bundled]);
console.log(`[artifact-helper] Prepared contained ${profile} DOCX/PDF builder for ${triple}.`);
