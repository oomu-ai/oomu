#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  statSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const tauriDirectory = join(root, "src-tauri");
const config = JSON.parse(readFileSync(join(tauriDirectory, "tauri.conf.json"), "utf8"));
const externalBins = config.bundle?.externalBin ?? [];
const verify = process.argv.includes("--verify");

function hostTriple() {
  const result = spawnSync("rustc", ["--print", "host-tuple"], {
    cwd: root,
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || "rustc host-tuple failed");
  return result.stdout.trim();
}

function preparedPaths() {
  const suffix = process.platform === "win32" ? ".exe" : "";
  const triple = hostTriple();
  return externalBins.map((stem) => join(tauriDirectory, `${stem}-${triple}${suffix}`));
}

function isMachO(bytes) {
  const magic = bytes.subarray(0, 4).toString("hex");
  return new Set(["cffaedfe", "feedfacf", "cafebabe", "bebafeca", "cafebabf", "bfbafeca"]).has(magic);
}

function reserve(paths) {
  for (const executable of paths) {
    mkdirSync(dirname(executable), { recursive: true });
    if (!existsSync(executable)) closeSync(openSync(executable, "wx", 0o755));
    if (process.platform !== "win32") chmodSync(executable, 0o755);
  }
  console.log(`[external-bins] Reserved ${paths.length} Tauri manifest path(s) for genuine compilation.`);
}

function verifyPrepared(paths) {
  for (const executable of paths) {
    const metadata = statSync(executable);
    if (!metadata.isFile() || metadata.size === 0) {
      throw new Error(`External binary was not genuinely prepared: ${executable}`);
    }
    if (process.platform !== "win32" && (metadata.mode & 0o111) === 0) {
      throw new Error(`External binary is not executable: ${executable}`);
    }
    if (process.platform === "darwin") {
      const bytes = readFileSync(executable);
      if (bytes.length < 4_096 || !isMachO(bytes)) {
        throw new Error(`External binary is not a substantive Mach-O executable: ${executable}`);
      }
    }
  }
  console.log(`[external-bins] Verified ${paths.length} genuinely compiled external binary file(s).`);
}

const paths = preparedPaths();
if (verify) verifyPrepared(paths);
else reserve(paths);
