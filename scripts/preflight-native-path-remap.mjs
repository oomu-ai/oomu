#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import {
  canonicalNativePathRemapConfiguration,
  canonicalNativePathRemapEnvironment,
} from "./release-environment.mjs";

const root = resolve(import.meta.dirname, "..");

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!["--clang", "--output"].includes(flag) || !value) {
      throw new Error(
        "Usage: preflight-native-path-remap.mjs --clang <clang> --output <report.json>",
      );
    }
    values[flag.slice(2)] = value;
  }
  if (!values.clang || !values.output) {
    throw new Error("Native path-remap preflight requires --clang and --output.");
  }
  return values;
}

function compileCanary({ clang, environmentFlags, language, source, output }) {
  const result = spawnSync(
    clang,
    [...environmentFlags, "-x", language, "-c", source, "-o", output],
    {
      cwd: root,
      encoding: "utf8",
      env: process.env,
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${basename(clang)} could not compile the ${language} path-remap canary: `
        + `${(result.stderr || result.stdout || "no output").trim()}`,
    );
  }
}

export function inspectNativePathRemapArtifact(bytes, configuration) {
  const forbidden = ["/Users/", "/System/Volumes/Data/Users/"];
  for (const [from] of configuration.mappings) forbidden.push(from);
  const hasLocalPath = [...new Set(forbidden)].some(
    (marker) => marker !== "/" && bytes.includes(Buffer.from(marker, "utf8")),
  );
  const findings = hasLocalPath ? ["local_path_marker"] : [];
  return { findings };
}

export function runNativePathRemapPreflight({ clang, outputPath }) {
  const canonicalClang = realpathSync(resolve(clang));
  const configuration = canonicalNativePathRemapConfiguration(root, process.env);
  const expectedEnvironment = canonicalNativePathRemapEnvironment(root, process.env);
  for (const name of ["CARGO_ENCODED_RUSTFLAGS", "CFLAGS", "CXXFLAGS"]) {
    if (process.env[name] !== expectedEnvironment[name]) {
      throw new Error(`Canonical native path-remap environment is missing ${name}.`);
    }
  }

  const workingDirectory = mkdtempSync(
    join(root, "src-tauri", "target", "native-path-remap-preflight-"),
  );
  const records = [];
  try {
    for (const language of ["c", "c++"]) {
      const extension = language === "c" ? "c" : "cpp";
      const source = join(workingDirectory, `canary.${extension}`);
      const object = join(workingDirectory, `canary-${extension}.o`);
      writeFileSync(
        source,
        "const char *oomu_native_path_remap_canary(void) { return __FILE__; }\n",
        { mode: 0o600 },
      );
      compileCanary({
        clang: canonicalClang,
        environmentFlags: process.env[language === "c" ? "CFLAGS" : "CXXFLAGS"]
          .split(/[\t\n\r ]+/u).filter(Boolean),
        language,
        source,
        output: object,
      });
      const bytes = readFileSync(object);
      const inspection = inspectNativePathRemapArtifact(bytes, configuration);
      if (inspection.findings.length > 0) {
        throw new Error(`${language} compiler embedded a local build path in its canary.`);
      }
      const repositoryMapping = configuration.mappings.find(
        ([from]) => from === root,
      );
      const canonicalSource = repositoryMapping
        ? join(repositoryMapping[1], relative(root, source)).split(sep).join("/")
        : null;
      if (!canonicalSource || !bytes.includes(Buffer.from(canonicalSource, "utf8"))) {
        throw new Error(`${language} compiler did not embed the canonical canary path.`);
      }
      records.push({
        language,
        object_sha256: sha256(bytes),
        canonical_path_sha256: sha256(Buffer.from(canonicalSource, "utf8")),
        local_path_findings: [],
      });
    }
    const report = {
      schema_version: 1,
      kind: "oomu.native-path-remap-preflight",
      status: "passed",
      synthetic: false,
      compiler_sha256: sha256(readFileSync(canonicalClang)),
      checked_languages: records,
    };
    writeFileSync(resolve(outputPath), `${JSON.stringify(report, null, 2)}\n`, {
      mode: 0o600,
    });
    return report;
  } finally {
    rmSync(workingDirectory, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const args = parseArguments(process.argv.slice(2));
    const report = runNativePathRemapPreflight({
      clang: args.clang,
      outputPath: args.output,
    });
    process.stdout.write(`${JSON.stringify(report)}\n`);
  } catch (error) {
    console.error(`NATIVE PATH-REMAP PREFLIGHT FAILED: ${error.message}`);
    process.exit(1);
  }
}
