import { spawnSync } from "node:child_process";
import { lstatSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

export const name = "macos_deployment_targets";
const MAXIMUM_MINIMUM_OS = [14, 0];

function execute(executable, args, allowFailure = false) {
  const result = spawnSync(executable, args, { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.error) throw result.error;
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (!allowFailure && result.status !== 0) {
    throw new Error(`${executable} failed while inspecting packaged code: ${output}`);
  }
  return { status: result.status ?? -1, output };
}

function discoverFiles(root) {
  const files = [];
  function visit(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = resolve(path, entry.name);
      const metadata = lstatSync(child);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) visit(child);
      else if (metadata.isFile()) files.push(child);
    }
  }
  visit(root);
  return files;
}

function versionTuple(value) {
  const parts = value.split(".").map((part) => Number.parseInt(part, 10));
  if (parts.length < 2 || parts.some((part) => !Number.isInteger(part))) return null;
  return [parts[0], parts[1]];
}

function supportedMinimum(value) {
  const tuple = versionTuple(value);
  return tuple &&
    (tuple[0] < MAXIMUM_MINIMUM_OS[0] ||
      (tuple[0] === MAXIMUM_MINIMUM_OS[0] && tuple[1] <= MAXIMUM_MINIMUM_OS[1]));
}

export function assessMachO({ architectures, builds, signatureStatus }) {
  const failures = [];
  if (!architectures.includes("arm64")) failures.push("arm64_missing");
  if (builds.length === 0) failures.push("build_version_missing");
  if (builds.some((build) => String(build.platform).toLowerCase() !== "macos")) {
    failures.push("platform_mismatch");
  }
  if (builds.some((build) => !build.minimumOs || !supportedMinimum(build.minimumOs))) {
    failures.push("minimum_os_incompatible");
  }
  if (signatureStatus !== 0) failures.push("signature_invalid");
  return failures;
}

export function parseBuildMetadata(output) {
  const builds = [];
  for (const block of String(output).split(/(?=Load command \d+|^architecture )/m)) {
    const platform = block.match(/^\s*platform\s+([^\s]+)/m)?.[1];
    const minimumOs = block.match(/^\s*minos\s+([0-9.]+)/m)?.[1];
    const sdk = block.match(/^\s*sdk\s+([0-9.]+)/m)?.[1];
    if (platform || minimumOs || sdk) builds.push({ platform, minimumOs, sdk });
  }
  return builds;
}

export async function run({ appPath, toolchain = {} }) {
  const canonicalApp = realpathSync(resolve(appPath));
  const lipo = toolchain.tools?.lipo?.executable ?? "/usr/bin/lipo";
  const codesign = toolchain.tools?.codesign?.executable ?? "/usr/bin/codesign";
  const vtool = execute("/usr/bin/xcrun", ["--find", "vtool"]).output.split(/\r?\n/).at(-1);
  const records = [];
  for (const path of discoverFiles(canonicalApp)) {
    const fileProbe = execute("/usr/bin/file", ["-b", path], true);
    if (fileProbe.status !== 0 || !fileProbe.output.includes("Mach-O")) continue;
    const architectures = execute(lipo, ["-archs", path]).output.split(/\s+/).filter(Boolean);
    const buildOutput = execute(vtool, ["-show-build", path]).output;
    const builds = parseBuildMetadata(buildOutput);
    const signature = execute(codesign, ["--verify", "--strict", "--verbose=2", path], true);
    const failures = assessMachO({
      architectures,
      builds,
      signatureStatus: signature.status,
    });
    records.push({
      path: relative(canonicalApp, path),
      architectures,
      builds,
      signature: signature.status === 0 ? "valid" : "invalid",
      failures,
    });
  }
  if (records.length === 0) throw new Error("No Mach-O executables were found in the app bundle.");
  const failed = records.filter((record) => record.failures.length > 0);
  if (failed.length > 0) {
    throw new Error(
      `Packaged native code does not support macOS 14: ${failed
        .map((record) => `${record.path} (${record.failures.join(",")})`)
        .join("; ")}`,
    );
  }
  return {
    passed: true,
    evidence: {
      schema_version: 1,
      baseline: "14.0",
      recursive: true,
      inspected_macho_count: records.length,
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
      throw new Error("Usage: macos-deployment-targets.mjs --app <OOMU.app> [--output report.json]");
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
    console.error(`MACOS DEPLOYMENT TARGET GATE FAILED: ${error.message}`);
    process.exit(1);
  }
}
