#!/usr/bin/env node

/* eslint-disable @typescript-eslint/no-require-imports */
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const tauriDir = path.join(root, "src-tauri");
const tauriConfigPath = path.join(tauriDir, "tauri.conf.json");
const portablePythonDir = path.join(tauriDir, "resources", "python");
const portablePythonLimitBytes = 25 * 1024 * 1024;
const maxReportedViolations = 80;

const pipLauncherPattern =
  /^pip(?:\d+(?:\.\d+)*)?(?:\.exe|-script\.py)?$/i;
const redundantPythonAliasPattern = /^python(?:3)?$/i;
const packageManagerDirectoryPatterns = [
  /^pip$/i,
  /^pip-.+\.dist-info$/i,
  /^ensurepip$/i,
  /^setuptools$/i,
  /^setuptools-.+\.dist-info$/i,
  /^_distutils_hack$/i,
  /^pkg_resources$/i,
];
const packageManagerFilePatterns = [/^distutils-precedence\.pth$/i];

function formatBytes(bytes) {
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function normalizeForDisplay(filePath) {
  return path.relative(root, filePath).split(path.sep).join("/");
}

function readTauriResources() {
  const config = JSON.parse(fs.readFileSync(tauriConfigPath, "utf8"));
  const resources = config.bundle?.resources;
  if (!resources) {
    return [];
  }
  if (Array.isArray(resources)) {
    return resources;
  }
  if (typeof resources === "object") {
    return Object.keys(resources);
  }
  throw new Error("bundle.resources must be an array or object.");
}

function hasGlobMagic(source) {
  return /[*?\[]/.test(source);
}

function resolveResourceSource(source) {
  return path.isAbsolute(source)
    ? path.normalize(source)
    : path.resolve(tauriDir, source);
}

function globStaticRoot(source) {
  const normalized = source.replace(/\\/g, "/");
  const segments = normalized.split("/");
  const rootSegments = [];

  for (const segment of segments) {
    if (/[*?\[]/.test(segment)) {
      break;
    }
    rootSegments.push(segment);
  }

  const staticPrefix = rootSegments.join("/") || ".";
  return resolveResourceSource(staticPrefix);
}

function configuredResourceRoots() {
  const roots = new Map();
  for (const source of readTauriResources()) {
    const resolved = hasGlobMagic(source)
      ? globStaticRoot(source)
      : resolveResourceSource(source);
    roots.set(path.normalize(resolved), source);
  }
  return [...roots.entries()].map(([resolved, source]) => ({ resolved, source }));
}

function pathSizeBytes(filePath) {
  const metadata = fs.lstatSync(filePath);
  if (!metadata.isDirectory()) {
    return metadata.size;
  }

  let size = metadata.size;
  for (const entry of fs.readdirSync(filePath)) {
    size += pathSizeBytes(path.join(filePath, entry));
  }
  return size;
}

function isPortablePythonPath(filePath) {
  const relative = path.relative(portablePythonDir, filePath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function debugArtifactReason(filePath, metadata) {
  const name = path.basename(filePath);
  if (metadata.isDirectory() && name === "__pycache__") {
    return "Python bytecode cache directory";
  }
  if (metadata.isFile() && name.endsWith(".pyc")) {
    return "Python bytecode cache file";
  }
  if (name === ".DS_Store") {
    return "macOS Finder metadata";
  }
  if (/\.log$/i.test(name)) {
    return "debug log";
  }
  if (/^\.env/i.test(name)) {
    return "environment file";
  }
  if (
    /\.(?:sqlite|sqlite3|db)$/i.test(name) &&
    /(?:^|[._-])(?:local|mock)(?:[._-]|$)/i.test(name)
  ) {
    return "local/mock SQLite database";
  }
  return null;
}

function packageManagerArtifactReason(filePath, metadata) {
  if (!isPortablePythonPath(filePath)) {
    return null;
  }

  const name = path.basename(filePath);
  const relative = path
    .relative(portablePythonDir, filePath)
    .split(path.sep)
    .join("/");

  if (
    metadata.isFile() &&
    /^(?:bin|Scripts)\//.test(relative) &&
    pipLauncherPattern.test(name)
  ) {
    return "pip launcher";
  }
  if (/^bin\//.test(relative) && redundantPythonAliasPattern.test(name)) {
    return "portable Python interpreter alias that Tauri dereferences into a duplicate binary";
  }
  if (
    metadata.isDirectory() &&
    packageManagerDirectoryPatterns.some((pattern) => pattern.test(name))
  ) {
    return "portable Python package-manager artifact";
  }
  if (
    metadata.isDirectory() &&
    /^lib\/python[^/]+\/unittest$/i.test(relative)
  ) {
    return "unused portable Python test-double framework";
  }
  if (
    metadata.isFile() &&
    packageManagerFilePatterns.some((pattern) => pattern.test(name))
  ) {
    return "portable Python package-manager artifact";
  }
  return null;
}

function scanForbiddenArtifacts(filePath, violations) {
  if (!fs.existsSync(filePath)) {
    violations.push({
      path: filePath,
      reason: "configured resource source does not exist",
    });
    return;
  }

  const metadata = fs.lstatSync(filePath);
  const reason =
    debugArtifactReason(filePath, metadata) ??
    packageManagerArtifactReason(filePath, metadata);
  if (reason) {
    violations.push({ path: filePath, reason });
    if (metadata.isDirectory()) {
      return;
    }
  }

  if (!metadata.isDirectory()) {
    return;
  }

  for (const entry of fs.readdirSync(filePath)) {
    scanForbiddenArtifacts(path.join(filePath, entry), violations);
  }
}

function assertPortablePythonSize(violations) {
  if (!fs.existsSync(portablePythonDir)) {
    violations.push({
      path: portablePythonDir,
      reason: "portable Python resource directory is missing",
    });
    return null;
  }

  const sizeBytes = pathSizeBytes(portablePythonDir);
  if (sizeBytes > portablePythonLimitBytes) {
    violations.push({
      path: portablePythonDir,
      reason: `portable Python is ${formatBytes(sizeBytes)}; limit is ${formatBytes(
        portablePythonLimitBytes,
      )}`,
    });
  }
  return sizeBytes;
}

function main() {
  const violations = [];
  const roots = configuredResourceRoots();
  const portablePythonSize = assertPortablePythonSize(violations);

  for (const { resolved } of roots) {
    scanForbiddenArtifacts(resolved, violations);
  }

  if (violations.length > 0) {
    console.error("[bundle-size] Resource bundle check failed:");
    for (const violation of violations.slice(0, maxReportedViolations)) {
      console.error(
        `  - ${violation.reason}: ${normalizeForDisplay(violation.path)}`,
      );
    }
    if (violations.length > maxReportedViolations) {
      console.error(
        `  - ... ${violations.length - maxReportedViolations} more violation(s) omitted`,
      );
    }
    process.exit(1);
  }

  const rootList = roots.map(({ source }) => source).join(", ");
  console.log(
    `[bundle-size] Portable Python ${formatBytes(
      portablePythonSize ?? 0,
    )} <= ${formatBytes(portablePythonLimitBytes)}.`,
  );
  console.log(`[bundle-size] Scanned bundle resources: ${rootList}`);
}

main();
