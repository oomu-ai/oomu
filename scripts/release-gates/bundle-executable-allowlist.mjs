import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { basename, dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const name = "bundle_executable_allowlist";
const defaultRoot = resolve(import.meta.dirname, "../..");

export const APPLICATION_EXECUTABLES = Object.freeze([
  "oomu",
  "local_infer",
  "pdf_extract_helper",
  "artifact_build_helper",
  "oomu-artifact-pdf-helper",
  "oomu-vision-helper",
  "oomu-speech-bridge",
]);

const INTERNAL_UTILITY_NAMES = new Set([
  "debug_db",
  "debug_executions",
  "oomu_bench",
  "stage_pre_alpha",
  "sanitize_release_db",
  "ark_verify",
]);

function execute(command, args, allowFailure = false) {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  if (result.error) throw result.error;
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (!allowFailure && result.status !== 0) {
    throw new Error(`${command} failed while inspecting the app bundle: ${output}`);
  }
  return { status: result.status ?? -1, output };
}

function walkFiles(root) {
  const files = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const child = resolve(directory, entry.name);
      const metadata = lstatSync(child);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) visit(child);
      else if (metadata.isFile()) files.push(child);
    }
  };
  visit(root);
  return files;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function signatureIdentity(path) {
  const result = execute("/usr/bin/codesign", ["-d", "--verbose=4", path], true);
  const authority = result.output.match(/^Authority=(.+)$/m)?.[1];
  const team = result.output.match(/^TeamIdentifier=(.+)$/m)?.[1];
  return {
    valid: execute("/usr/bin/codesign", ["--verify", "--strict", "--verbose=2", path], true).status === 0,
    identity: authority ?? team ?? null,
  };
}

function readPortablePythonManifest(files) {
  const manifests = files.filter((path) => basename(path) === ".oomu-python-native-manifest.json");
  if (manifests.length !== 1) {
    throw new Error(`Expected one portable-Python native-file manifest; found ${manifests.length}.`);
  }
  const manifest = JSON.parse(readFileSync(manifests[0], "utf8"));
  if (manifest.schema_version !== 1 || !Array.isArray(manifest.files)) {
    throw new Error("Portable-Python native-file manifest is malformed.");
  }
  const entries = new Map();
  for (const entry of manifest.files) {
    if (!entry || typeof entry.path !== "string" || !/^[a-f0-9]{64}$/u.test(entry.sha256 ?? "")) {
      throw new Error("Portable-Python native-file manifest contains an invalid entry.");
    }
    if (entry.path.startsWith("/") || entry.path.split("/").includes("..") || entries.has(entry.path)) {
      throw new Error(`Portable-Python native-file manifest has an unsafe or duplicate path: ${entry.path}`);
    }
    entries.set(entry.path, entry);
  }
  return { path: manifests[0], root: dirname(manifests[0]), entries };
}

export function validateBundleRecords(records, { pythonManifestEntries = new Map() } = {}) {
  const failures = [];
  const seenApplicationNames = new Map();
  const seenPythonPaths = new Set();
  for (const record of records) {
    if ((record.mode & 0o022) !== 0) failures.push(`${record.path}: executable is group/world writable`);
    if (INTERNAL_UTILITY_NAMES.has(record.name)) failures.push(`${record.path}: internal utility is forbidden`);
    if (record.rule === "application-executable") {
      if (seenApplicationNames.has(record.name)) {
        failures.push(`${record.path}: duplicate application helper name ${record.name}`);
      }
      seenApplicationNames.set(record.name, record.path);
      if (!record.path.startsWith("Contents/MacOS/")) {
        failures.push(`${record.path}: application executable is outside Contents/MacOS`);
      }
    } else if (record.rule === "portable-python-manifest") {
      const expected = pythonManifestEntries.get(record.pythonRelativePath);
      seenPythonPaths.add(record.pythonRelativePath);
      if (!expected) {
        failures.push(`${record.path}: portable-Python file is absent from its manifest`);
      } else if ((expected.kind === "mach-o") !== record.machO) {
        failures.push(`${record.path}: portable-Python file type differs from its manifest`);
      } else if (!record.machO && expected.sha256 !== record.sha256) {
        failures.push(`${record.path}: portable-Python executable differs from its manifest`);
      }
    } else {
      failures.push(`${record.path}: executable or native code is not allowlisted`);
    }
    if (record.machO && !record.signatureValid) failures.push(`${record.path}: Mach-O signature is invalid`);
  }
  for (const required of APPLICATION_EXECUTABLES) {
    if (!seenApplicationNames.has(required)) failures.push(`Contents/MacOS/${required}: required executable is missing`);
  }
  for (const requiredPath of pythonManifestEntries.keys()) {
    if (!seenPythonPaths.has(requiredPath)) {
      failures.push(`portable Python manifest entry is missing from bundle: ${requiredPath}`);
    }
  }
  return [...new Set(failures)].sort();
}

export function classifyBundleFile({ name: fileName, executable, machO, shebang, pythonRelativePath }) {
  if (APPLICATION_EXECUTABLES.includes(fileName)) return "application-executable";
  if (pythonRelativePath !== null) return "portable-python-manifest";
  if (executable || machO || shebang) return "unapproved";
  return null;
}

export async function run({ appPath, root = defaultRoot, toolchain = {} }) {
  const canonicalApp = realpathSync(resolve(appPath));
  const releasePolicy = JSON.parse(
    readFileSync(resolve(root, "release/pre_alpha/sanitizer_manifest.json"), "utf8"),
  ).executable_policy;
  if (
    JSON.stringify(releasePolicy?.application_executables) !== JSON.stringify(APPLICATION_EXECUTABLES) ||
    releasePolicy?.portable_python_manifest !== ".oomu-python-native-manifest.json"
  ) {
    throw new Error("Release executable policy and bundle allowlist are not identical.");
  }
  const files = walkFiles(canonicalApp);
  const portable = readPortablePythonManifest(files);
  const lipo = toolchain.tools?.lipo?.executable ?? "/usr/bin/lipo";
  const records = [];
  for (const path of files) {
    const metadata = lstatSync(path);
    const relativePath = relative(canonicalApp, path).split(sep).join("/");
    const fileType = execute("/usr/bin/file", ["-b", path], true).output;
    const machO = /Mach-O/u.test(fileType);
    const executable = (metadata.mode & 0o111) !== 0;
    const shebang = executable && readFileSync(path).subarray(0, 2).toString("utf8") === "#!";
    if (!machO && !executable) continue;
    const withinPython = path.startsWith(`${portable.root}${sep}`);
    const pythonRelativePath = withinPython
      ? relative(portable.root, path).split(sep).join("/")
      : null;
    const rule = classifyBundleFile({
      relativePath,
      name: basename(path),
      executable,
      machO,
      shebang,
      pythonRelativePath,
    });
    const signature = machO ? signatureIdentity(path) : { valid: null, identity: null };
    records.push({
      path: relativePath,
      name: basename(path),
      fileType,
      architectures: machO
        ? execute(lipo, ["-archs", path], true).output.split(/\s+/u).filter(Boolean)
        : [],
      sha256: sha256(path),
      mode: metadata.mode & 0o777,
      executable,
      machO,
      shebang,
      signatureValid: signature.valid,
      signatureIdentity: signature.identity,
      rule,
      pythonRelativePath,
    });
  }
  records.sort((left, right) => left.path.localeCompare(right.path));
  const failures = validateBundleRecords(records, { pythonManifestEntries: portable.entries });
  if (failures.length > 0) {
    throw new Error(`Bundle executable policy failed: ${failures.join("; ")}`);
  }
  return {
    passed: true,
    evidence: {
      schema_version: 1,
      recursive: true,
      app_path: canonicalApp,
      portable_python_manifest: relative(canonicalApp, portable.path).split(sep).join("/"),
      allowed_application_executables: APPLICATION_EXECUTABLES,
      inspected_entry_count: records.length,
      records,
    },
  };
}

function cliArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!["--app", "--output"].includes(flag) || !value) {
      throw new Error("Usage: bundle-executable-allowlist.mjs --app <OOMU.app> [--output report.json]");
    }
    values[flag.slice(2)] = value;
  }
  if (!values.app) throw new Error("--app is required.");
  return values;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const args = cliArguments(process.argv.slice(2));
    const result = await run({ appPath: args.app });
    const encoded = `${JSON.stringify(result, null, 2)}\n`;
    if (args.output) writeFileSync(resolve(args.output), encoded, { mode: 0o600 });
    process.stdout.write(encoded);
  } catch (error) {
    console.error(`BUNDLE EXECUTABLE ALLOWLIST GATE FAILED: ${error.message}`);
    process.exit(1);
  }
}
