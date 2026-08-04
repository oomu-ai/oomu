#!/usr/bin/env node

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { run } from "./release-gates/build-path-privacy.mjs";

function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!["--app", "--output"].includes(flag) || !value) {
      throw new Error("Usage: check-build-path-privacy.mjs --app <OOMU.app> --output <report.json>");
    }
    values[flag.slice(2)] = value;
  }
  if (!values.app || !values.output) {
    throw new Error("Build-path privacy check requires --app and --output.");
  }
  return values;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const args = parseArguments(process.argv.slice(2));
    const result = await run({
      root: resolve(import.meta.dirname, ".."),
      appPath: resolve(args.app),
    });
    writeFileSync(resolve(args.output), `${JSON.stringify(result, null, 2)}\n`, {
      mode: 0o600,
    });
  } catch (error) {
    console.error(`BUILD-PATH PRIVACY CHECK FAILED: ${error.message}`);
    process.exit(1);
  }
}
