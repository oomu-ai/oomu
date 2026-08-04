import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const resourcePythonDir = join(root, "src-tauri", "resources", "python");
const cacheDir = join(root, "src-tauri", "resources", ".python-cache");
const release = process.env.OOMU_PORTABLE_PYTHON_RELEASE ?? "20260623";
const version = process.env.OOMU_PORTABLE_PYTHON_VERSION ?? "3.10.20";
const pythonMajorMinor = version.split(".").slice(0, 2).join(".");
const pipLauncherPattern =
  /^pip(?:\d+(?:\.\d+)*)?(?:\.exe|-script\.py)?$/i;
const devToolLauncherPattern =
  /^(?:2to3|idle|pydoc)(?:-?\d+(?:\.\d+)*)?(?:\.exe|-script\.py)?$|^python(?:\d+(?:\.\d+)*)?-config(?:\.exe)?$/i;
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

const knownSha256 = new Map([
  [
    "cpython-3.10.20+20260623-aarch64-apple-darwin-install_only.tar.gz",
    "12404cce3ae7b72b58491dfd6289ac93e838a4a49c45666e25ffee86c5889dc2",
  ],
  [
    "cpython-3.10.20+20260623-aarch64-unknown-linux-gnu-install_only.tar.gz",
    "1387d63c1c5797063bbb38824a6be274030d65e4a67fa88e8d9eb3252e9b7ce2",
  ],
]);

function portablePythonBinary(rootDir) {
  return process.platform === "win32"
    ? join(rootDir, "Scripts", "python.exe")
    : join(rootDir, "bin", `python${pythonMajorMinor}`);
}

function targetTriple() {
  if (process.platform === "darwin" && process.arch === "arm64") {
    return "aarch64-apple-darwin";
  }
  if (process.platform === "darwin" && process.arch === "x64") {
    return "x86_64-apple-darwin";
  }
  if (process.platform === "linux" && process.arch === "arm64") {
    return "aarch64-unknown-linux-gnu";
  }
  if (process.platform === "linux" && process.arch === "x64") {
    return "x86_64-unknown-linux-gnu";
  }
  throw new Error(
    `No bundled Python target mapping for ${process.platform}/${process.arch}. Set OOMU_PORTABLE_PYTHON_URL to override.`,
  );
}

function defaultAssetName() {
  return `cpython-${version}+${release}-${targetTriple()}-install_only.tar.gz`;
}

const overrideUrl = process.env.OOMU_PORTABLE_PYTHON_URL;
const assetName =
  process.env.OOMU_PORTABLE_PYTHON_ASSET ??
  (overrideUrl ? decodeURIComponent(basename(new URL(overrideUrl).pathname)) : defaultAssetName());
const assetUrl =
  overrideUrl ??
  `https://github.com/astral-sh/python-build-standalone/releases/download/${release}/${assetName.replace(/\+/g, "%2B")}`;
const expectedSha256 =
  process.env.OOMU_PORTABLE_PYTHON_SHA256 ?? knownSha256.get(assetName);
const tarballPath = join(cacheDir, basename(assetName));
const stagingDir = join(cacheDir, "staging");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function downloadTarball() {
  mkdirSync(cacheDir, { recursive: true });
  if (existsSync(tarballPath)) {
    console.log(`[portable-python] Reusing cached tarball ${tarballPath}.`);
    return;
  }

  console.log(`[portable-python] Downloading standalone Python from ${assetUrl}`);
  const curl = spawnSync("curl", ["-fL", assetUrl, "-o", tarballPath], {
    cwd: root,
    stdio: "inherit",
    env: process.env,
  });
  if (curl.status === 0) {
    return;
  }

  const wget = spawnSync("wget", ["-O", tarballPath, assetUrl], {
    cwd: root,
    stdio: "inherit",
    env: process.env,
  });
  if (wget.error) {
    throw wget.error;
  }
  if (wget.status !== 0) {
    process.exit(wget.status ?? 1);
  }
}

function verifyTarball() {
  if (!expectedSha256) {
    throw new Error(
      `Portable Python has no pinned SHA-256 for ${assetName}; refusing to prepare an unverified runtime.`,
    );
  }
  const digest = createHash("sha256").update(readFileSync(tarballPath)).digest("hex");
  if (digest !== expectedSha256) {
    throw new Error(
      `Portable Python checksum mismatch for ${assetName}: expected ${expectedSha256}, got ${digest}`,
    );
  }
}

function findPythonRoot(startDir, depth = 0) {
  if (existsSync(portablePythonBinary(startDir))) {
    return startDir;
  }
  if (depth >= 4) {
    return null;
  }
  for (const entry of readdirSync(startDir)) {
    const child = join(startDir, entry);
    if (statSync(child).isDirectory()) {
      const found = findPythonRoot(child, depth + 1);
      if (found) {
        return found;
      }
    }
  }
  return null;
}

function copyDirectoryContents(source, destination) {
  mkdirSync(destination, { recursive: true });
  for (const entry of readdirSync(source)) {
    cpSync(join(source, entry), join(destination, entry), {
      recursive: true,
      force: true,
      errorOnExist: false,
      verbatimSymlinks: true,
    });
  }
}

function clearGeneratedPythonRuntime(destination) {
  mkdirSync(destination, { recursive: true });
  for (const entry of readdirSync(destination)) {
    rmSync(join(destination, entry), { recursive: true, force: true });
  }
}

function removeIfExists(path) {
  rmSync(path, { recursive: true, force: true });
}

function removeMatchingChildren(directory, matcher) {
  if (!existsSync(directory)) {
    return;
  }
  for (const entry of readdirSync(directory)) {
    if (matcher(entry)) {
      removeIfExists(join(directory, entry));
    }
  }
}

function prunePythonCaches(directory) {
  if (!existsSync(directory)) {
    return;
  }

  for (const entry of readdirSync(directory)) {
    const child = join(directory, entry);
    const metadata = lstatSync(child);

    if (metadata.isDirectory()) {
      if (entry === "__pycache__") {
        removeIfExists(child);
      } else {
        prunePythonCaches(child);
      }
      continue;
    }

    if (metadata.isFile() && entry.endsWith(".pyc")) {
      removeIfExists(child);
    }
  }
}

function prunePackageManagerArtifacts(directory) {
  if (!existsSync(directory)) {
    return;
  }

  for (const entry of readdirSync(directory)) {
    const child = join(directory, entry);
    const metadata = lstatSync(child);

    if (metadata.isDirectory()) {
      if (packageManagerDirectoryPatterns.some((pattern) => pattern.test(entry))) {
        removeIfExists(child);
      } else {
        prunePackageManagerArtifacts(child);
      }
      continue;
    }

    if (
      metadata.isFile() &&
      packageManagerFilePatterns.some((pattern) => pattern.test(entry))
    ) {
      removeIfExists(child);
    }
  }
}

function removePipLaunchers(pythonRoot) {
  for (const relative of ["bin", "Scripts"]) {
    removeMatchingChildren(join(pythonRoot, relative), (entry) =>
      pipLauncherPattern.test(entry),
    );
  }
}

function removeDevToolLaunchers(pythonRoot) {
  for (const relative of ["bin", "Scripts"]) {
    removeMatchingChildren(join(pythonRoot, relative), (entry) =>
      devToolLauncherPattern.test(entry),
    );
  }
}

function removeInterpreterAliases(pythonRoot) {
  if (process.platform === "win32") {
    return;
  }
  for (const entry of ["python", "python3"]) {
    removeIfExists(join(pythonRoot, "bin", entry));
  }
}

function pythonSmokeTest(pythonRoot) {
  const pythonBinary = portablePythonBinary(pythonRoot);
  if (process.platform !== "win32" && existsSync(pythonBinary)) {
    chmodSync(pythonBinary, 0o755);
  }

  const result = spawnSync(
    pythonBinary,
    [
      "-c",
      "import datetime, html.parser, json, os, ssl, subprocess, sys, urllib.request",
    ],
    {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    },
  );
  return !result.error && result.status === 0;
}

function removeUnusedSharedPythonLibraries(pythonRoot) {
  const libRoot = join(pythonRoot, "lib");
  if (!existsSync(libRoot)) {
    return;
  }

  for (const entry of readdirSync(libRoot)) {
    if (!/^libpython.+\.(?:dylib|so(?:\.\d+)*)$/i.test(entry)) {
      continue;
    }

    const candidate = join(libRoot, entry);
    const backup = `${candidate}.prune-candidate`;
    removeIfExists(backup);
    renameSync(candidate, backup);
    if (pythonSmokeTest(pythonRoot)) {
      removeIfExists(backup);
      continue;
    }
    renameSync(backup, candidate);
  }
}

function stripUnneededArtifacts(pythonRoot) {
  for (const relative of [
    "include",
    "share/doc",
    "share/man",
    "lib/pkgconfig",
  ]) {
    removeIfExists(join(pythonRoot, relative));
  }

  const libRoot = join(pythonRoot, "lib");
  if (!existsSync(libRoot)) {
    return;
  }
  removeMatchingChildren(libRoot, (entry) =>
    /^(?:itcl|tcl|tk|thread)\d|^libtcl|^libtk/i.test(entry),
  );
  for (const entry of readdirSync(libRoot)) {
    if (!entry.startsWith("python")) {
      continue;
    }
    for (const relative of [
      "distutils",
      "idlelib",
      "lib2to3",
      "test",
      "tkinter",
      "turtledemo",
      "unittest",
    ]) {
      removeIfExists(join(libRoot, entry, relative));
    }
    removeMatchingChildren(join(libRoot, entry), (childEntry) =>
      childEntry.startsWith("config-"),
    );
    removeMatchingChildren(join(libRoot, entry, "lib-dynload"), (childEntry) =>
      childEntry.startsWith("_tkinter."),
    );
  }
}

function prunePortablePython(pythonRoot) {
  stripUnneededArtifacts(pythonRoot);
  removePipLaunchers(pythonRoot);
  removeDevToolLaunchers(pythonRoot);
  removeInterpreterAliases(pythonRoot);
  prunePackageManagerArtifacts(pythonRoot);
  prunePythonCaches(pythonRoot);
  removeUnusedSharedPythonLibraries(pythonRoot);
  prunePythonCaches(pythonRoot);
}

function isMachO(bytes) {
  if (bytes.length < 4) return false;
  const magic = bytes.readUInt32BE(0);
  return [0xfeedface, 0xfeedfacf, 0xcefaedfe, 0xcffaedfe, 0xcafebabe, 0xbebafeca]
    .includes(magic);
}

function portableRuntimeFiles(directory) {
  const files = [];
  const visit = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (entry.isFile()) files.push(path);
    }
  };
  visit(directory);
  return files;
}

function writeNativeFileManifest(pythonRoot) {
  const files = portableRuntimeFiles(pythonRoot)
    .filter((path) => basename(path) !== ".oomu-python-native-manifest.json")
    .flatMap((path) => {
      const metadata = lstatSync(path);
      const bytes = readFileSync(path);
      const macho = isMachO(bytes.subarray(0, 4));
      const executable = (metadata.mode & 0o111) !== 0;
      if (!macho && !executable) return [];
      return [{
        path: relative(pythonRoot, path).split(sep).join("/"),
        sha256: createHash("sha256").update(bytes).digest("hex"),
        kind: macho ? "mach-o" : bytes.subarray(0, 2).toString("utf8") === "#!" ? "script" : "executable-data",
        executable,
      }];
    })
    .sort((left, right) => left.path.localeCompare(right.path));
  writeFileSync(
    join(pythonRoot, ".oomu-python-native-manifest.json"),
    `${JSON.stringify({
      schema_version: 1,
      source: "oomu-portable-python-import-runtime",
      required_imports: ["datetime", "html.parser", "json", "os", "ssl", "subprocess", "sys", "urllib.request"],
      files,
    }, null, 2)}\n`,
    { mode: 0o644 },
  );
}

if (existsSync(portablePythonBinary(resourcePythonDir))) {
  prunePortablePython(resourcePythonDir);
  writeNativeFileManifest(resourcePythonDir);
  console.log("[portable-python] Standalone environment is already cached and pruned.");
  process.exit(0);
}

downloadTarball();
verifyTarball();

rmSync(stagingDir, { recursive: true, force: true });
mkdirSync(stagingDir, { recursive: true });
run("tar", ["-xzf", tarballPath, "-C", stagingDir]);

const extractedPythonRoot = findPythonRoot(stagingDir);
if (!extractedPythonRoot) {
  throw new Error(`Extracted standalone Python did not contain bin/python${pythonMajorMinor}.`);
}

clearGeneratedPythonRuntime(resourcePythonDir);
copyDirectoryContents(extractedPythonRoot, resourcePythonDir);
prunePortablePython(resourcePythonDir);

const pythonBinary = portablePythonBinary(resourcePythonDir);
if (process.platform !== "win32") {
  chmodSync(pythonBinary, 0o755);
}

writeFileSync(
  join(resourcePythonDir, ".oomu-python-build.json"),
  `${JSON.stringify(
    {
      source: "astral-sh/python-build-standalone",
      release,
      asset: assetName,
      url: assetUrl,
      sha256: expectedSha256 ?? null,
    },
    null,
    2,
  )}\n`,
);

rmSync(stagingDir, { recursive: true, force: true });
run(pythonBinary, ["--version"]);
prunePythonCaches(resourcePythonDir);
writeNativeFileManifest(resourcePythonDir);
console.log(`[portable-python] Cached standalone Python at ${resourcePythonDir}.`);
