import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  APPLICATION_EXECUTABLES,
  classifyBundleFile,
  validateBundleRecords,
} from "../release-gates/bundle-executable-allowlist.mjs";

const root = resolve(import.meta.dirname, "../..");

describe("bundle_executable_allowlist release gate", () => {
  it("contains exactly the reviewed application executables", () => {
    expect(APPLICATION_EXECUTABLES).toEqual([
      "oomu", "local_infer", "pdf_extract_helper", "artifact_build_helper",
      "oomu-artifact-pdf-helper", "oomu-vision-helper", "oomu-speech-bridge",
    ]);
  });

  it("rejects an unknown executable and every internal utility", () => {
    const records = APPLICATION_EXECUTABLES.map((name) => ({
      path: `Contents/MacOS/${name}`, name, mode: 0o755, machO: true,
      signatureValid: true, rule: "application-executable",
    }));
    records.push({
      path: "Contents/MacOS/debug_db", name: "debug_db", mode: 0o755,
      machO: true, signatureValid: true, rule: "unapproved",
    });
    expect(validateBundleRecords(records)).toEqual(expect.arrayContaining([
      expect.stringContaining("internal utility is forbidden"),
      expect.stringContaining("not allowlisted"),
    ]));
  });

  it("requires portable native files to match the generated file manifest", () => {
    const records = APPLICATION_EXECUTABLES.map((name) => ({
      path: `Contents/MacOS/${name}`, name, mode: 0o755, machO: true,
      signatureValid: true, rule: "application-executable",
    }));
    records.push({
      path: "Contents/Resources/resources/python/bin/python3.10",
      name: "python3.10", mode: 0o755, machO: false, signatureValid: null,
      rule: "portable-python-manifest", pythonRelativePath: "bin/python3.10", sha256: "actual",
    });
    const failures = validateBundleRecords(records, {
      pythonManifestEntries: new Map([["bin/python3.10", { sha256: "reviewed", kind: "executable-data" }]]),
    });
    expect(failures).toContainEqual(expect.stringContaining("executable differs from its manifest"));
  });

  it("isolates all developer binaries from the Tauri Cargo package", () => {
    const cargo = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8");
    const developerCargo = readFileSync(
      resolve(root, "tools/developer-tools/Cargo.toml"),
      "utf8",
    );
    for (const name of ["debug_db", "debug_executions", "oomu_bench", "stage_pre_alpha", "sanitize_release_db", "ark_verify"]) {
      expect(cargo).not.toContain(`name = "${name}"`);
      expect(developerCargo).toContain(`name = "${name}"`);
    }
    expect(classifyBundleFile({ name: "debug_db", executable: true, machO: true, shebang: false, pythonRelativePath: null })).toBe("unapproved");
  });
});
