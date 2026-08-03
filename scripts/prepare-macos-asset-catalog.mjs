#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { join, resolve } from "node:path";
import process from "node:process";

import { assertAppleDeveloperToolPath } from "./preflight-apple-toolchain.mjs";

const root = resolve(import.meta.dirname, "..");
const tauriRoot = join(root, "src-tauri");
const sourceIcon = join(tauriRoot, "icons", "OOMU.icon");
const targetRoot = join(tauriRoot, "target");
const outputDirectory = join(targetRoot, "macos-asset-catalog");

function run(executable, args, label) {
  const result = spawnSync(executable, args, {
    cwd: root,
    encoding: "utf8",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${label} failed${detail ? `:\n${detail}` : "."}`);
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
}

function requireNonemptyFile(path, label) {
  if (!existsSync(path) || !statSync(path).isFile() || statSync(path).size === 0) {
    throw new Error(`${label} was not generated as a non-empty file.`);
  }
}

export function prepareMacosAssetCatalog() {
  if (process.platform !== "darwin") {
    throw new Error("The production macOS asset catalog requires macOS.");
  }
  if (!existsSync(sourceIcon)) {
    throw new Error(`The canonical icon source is missing: ${sourceIcon}`);
  }

  const tauri = JSON.parse(readFileSync(join(tauriRoot, "tauri.conf.json"), "utf8"));
  const minimumSystemVersion = tauri.bundle?.macOS?.minimumSystemVersion;
  const bundleIdentifier = tauri.identifier;
  if (!/^\d+\.\d+$/u.test(minimumSystemVersion ?? "") || !bundleIdentifier) {
    throw new Error("The Tauri macOS deployment target or bundle identifier is invalid.");
  }

  const developerDirectory = realpathSync(run(
    "/usr/bin/xcode-select",
    ["-p"],
    "Resolving the active Xcode developer directory",
  ));
  const actool = assertAppleDeveloperToolPath(
    "actool",
    run("/usr/bin/xcrun", ["--find", "actool"], "Resolving actool"),
    developerDirectory,
  );

  mkdirSync(targetRoot, { recursive: true });
  const temporaryDirectory = mkdtempSync(join(targetRoot, ".macos-asset-catalog-"));
  try {
    const partialInfoPlist = join(temporaryDirectory, "partial-info.plist");
    run(actool, [
      sourceIcon,
      "--compile", temporaryDirectory,
      "--output-format", "human-readable-text",
      "--notices",
      "--warnings",
      "--platform", "macosx",
      "--minimum-deployment-target", minimumSystemVersion,
      "--app-icon", "OOMU",
      "--enable-on-demand-resources", "NO",
      "--development-region", "en",
      "--target-device", "mac",
      "--bundle-identifier", bundleIdentifier,
      "--output-partial-info-plist", partialInfoPlist,
    ], "Compiling the canonical macOS asset catalog");

    const assetsPath = join(temporaryDirectory, "Assets.car");
    const iconPath = join(temporaryDirectory, "OOMU.icns");
    requireNonemptyFile(assetsPath, "Assets.car");
    requireNonemptyFile(iconPath, "OOMU.icns");
    requireNonemptyFile(partialInfoPlist, "The partial icon Info.plist");

    const catalog = JSON.parse(run(
      "/usr/bin/assetutil",
      ["--info", assetsPath],
      "Validating Assets.car",
    ));
    if (!Array.isArray(catalog) || catalog.length === 0) {
      throw new Error("Assets.car contains no structurally valid asset entries.");
    }

    rmSync(outputDirectory, { recursive: true, force: true });
    renameSync(temporaryDirectory, outputDirectory);
    process.stdout.write(
      `[macos-assets] Compiled ${catalog.length} canonical asset entries.\n`,
    );
    return { assetCount: catalog.length, outputDirectory };
  } catch (error) {
    rmSync(temporaryDirectory, { recursive: true, force: true });
    throw error;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(import.meta.filename)) {
  prepareMacosAssetCatalog();
}
