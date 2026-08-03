import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import process from "node:process";

const defaultRoot = resolve(import.meta.dirname, "..");

function jsonSerializable(value) {
  try {
    JSON.stringify(value);
    return true;
  } catch {
    return false;
  }
}

export function discoverReleaseGateFiles(gateDirectory) {
  return readdirSync(gateDirectory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".mjs"))
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));
}

export function validateReleaseGateResult(name, result) {
  if (
    !result ||
    result.passed !== true ||
    typeof result.evidence !== "object" ||
    result.evidence === null ||
    !jsonSerializable(result)
  ) {
    throw new Error(`Release extension ${name} did not produce valid passing evidence.`);
  }
  return result;
}

export async function runReleaseExtensionGates({
  root = defaultRoot,
  appPath,
  evidenceDir,
  toolchain = {},
}) {
  const gateDirectory = join(root, "scripts", "release-gates");
  const modules = discoverReleaseGateFiles(gateDirectory);
  const names = new Set();
  const results = [];
  for (const fileName of modules) {
    const gateModule = await import(pathToFileURL(join(gateDirectory, fileName)).href);
    if (!/^[a-z][a-z0-9_]{2,63}$/.test(gateModule.name ?? "") || names.has(gateModule.name)) {
      throw new Error(`Release extension ${fileName} has a missing, invalid, or duplicate name.`);
    }
    if (typeof gateModule.run !== "function") {
      throw new Error(`Release extension ${gateModule.name} does not export run().`);
    }
    names.add(gateModule.name);
    const result = await gateModule.run({ root, appPath, evidenceDir, toolchain });
    validateReleaseGateResult(gateModule.name, result);
    results.push({ name: gateModule.name, ...result });
  }
  return {
    schema_version: 1,
    status: "passed",
    synthetic: false,
    lexical_order: true,
    gate_count: results.length,
    results,
  };
}

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!["--app", "--evidence-dir", "--toolchain", "--output"].includes(key) || !value) {
      throw new Error(
        "Usage: run-release-extension-gates.mjs --app <OOMU.app> --evidence-dir <dir> --toolchain <toolchain.json> --output <report.json>",
      );
    }
    values[key.slice(2)] = value;
  }
  for (const required of ["app", "evidence-dir", "toolchain", "output"]) {
    if (!values[required]) throw new Error(`--${required} is required.`);
  }
  return values;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const args = parseArgs(process.argv.slice(2));
    const toolchain = JSON.parse(readFileSync(args.toolchain, "utf8"));
    const result = await runReleaseExtensionGates({
      appPath: args.app,
      evidenceDir: args["evidence-dir"],
      toolchain,
    });
    writeFileSync(resolve(args.output), `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
    process.stdout.write(`${JSON.stringify(result)}\n`);
  } catch (error) {
    console.error(`RELEASE EXTENSION GATES FAILED: ${error.message}`);
    process.exit(1);
  }
}
