#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import process from "node:process";
import { atomicWriteFile } from "./release-manifest.mjs";

const root = resolve(import.meta.dirname, "..");
const manifestPath = join(root, "tools", "developer-tools", "Cargo.toml");

function parseArgs(argv) {
  const options = { release: false, dir: null, evidence: null, buildId: null };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--release") {
      options.release = true;
      continue;
    }
    if (["--dir", "--evidence", "--build-id"].includes(arg)) {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value.`);
      const key = arg === "--build-id" ? "buildId" : arg.slice(2);
      options[key] = key === "buildId" ? value : resolve(value);
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }
  if ((options.evidence || options.buildId) && (!options.evidence || !options.buildId)) {
    throw new Error("--evidence and --build-id must be provided together.");
  }
  return options;
}

function isDatabasePath(path) {
  const lower = path.toLowerCase();
  return lower.endsWith(".sqlite") || lower.endsWith(".sqlite3") || lower.endsWith(".db");
}

function scanDatabases(directory) {
  const databases = [];
  const prohibitedSymbolicLinks = [];
  let filesScanned = 0;
  let symbolicLinksSkipped = 0;
  function walk(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const entryPath = join(path, entry.name);
      if (entry.isSymbolicLink()) {
        symbolicLinksSkipped += 1;
        if (isDatabasePath(entry.name)) prohibitedSymbolicLinks.push(entryPath);
      } else if (entry.isDirectory()) {
        walk(entryPath);
      } else if (entry.isFile()) {
        filesScanned += 1;
        if (isDatabasePath(entry.name)) databases.push(entryPath);
      }
    }
  }
  walk(directory);
  databases.sort();
  return { databases, prohibitedSymbolicLinks, filesScanned, symbolicLinksSkipped };
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function sanitizeDatabase(dbPath, release) {
  const cargoArgs = [
    "run",
    "--manifest-path",
    manifestPath,
    "--bin",
    "sanitize_release_db",
    "--features",
    "database-tools",
  ];
  if (release) cargoArgs.push("--release");
  cargoArgs.push("--", dbPath);
  console.log(`[release-db] Sanitizing ${dbPath}`);
  const result = spawnSync("cargo", cargoArgs, {
    cwd: root,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Database sanitizer failed for ${dbPath} with exit ${result.status}.`);
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  let target = options.dir;
  let scan;
  if (target) {
    if (!existsSync(target) || !lstatSync(target).isDirectory()) {
      throw new Error(`Database sanitation target is not a directory: ${target}`);
    }
    target = realpathSync(target);
    scan = scanDatabases(target);
  } else {
    const candidates = process.env.OOMU_RELEASE_DB_PATH
      ? [resolve(process.env.OOMU_RELEASE_DB_PATH)]
      : [
          join(root, "src-tauri", "oomu_state.sqlite"),
          join(root, "oomu_state.sqlite"),
          join(root, "app_data", "oomu_state.sqlite"),
        ];
    const databases = candidates.filter(
      (candidate) => existsSync(candidate) && lstatSync(candidate).isFile(),
    );
    scan = {
      databases,
      prohibitedSymbolicLinks: [],
      filesScanned: databases.length,
      symbolicLinksSkipped: 0,
    };
    target = root;
  }

  if (scan.prohibitedSymbolicLinks.length > 0) {
    throw new Error(
      `Database-like path is a symbolic link and cannot be attested: ${scan.prohibitedSymbolicLinks[0]}`,
    );
  }

  const results = [];
  for (const dbPath of scan.databases) {
    sanitizeDatabase(dbPath, options.release);
    const stats = lstatSync(dbPath);
    results.push({
      path: relative(target, dbPath).split(sep).join("/"),
      size_bytes: stats.size,
      sha256: sha256File(dbPath),
      status: "sanitized",
    });
  }

  if (scan.databases.length === 0) {
    console.log("[release-db] Scan completed: no database artifact is present in the release tree.");
  } else {
    console.log(`[release-db] Sanitized ${scan.databases.length} database artifact(s).`);
  }
  if (options.evidence) {
    const report = {
      schema_version: 1,
      kind: "oomu.release-database-sanitizer-execution",
      status: "passed",
      synthetic: false,
      build_identifier: options.buildId,
      completed_at: new Date().toISOString(),
      target,
      files_scanned: scan.filesScanned,
      symbolic_links_skipped: scan.symbolicLinksSkipped,
      database_count: scan.databases.length,
      databases: results,
    };
    atomicWriteFile(options.evidence, `${JSON.stringify(report, null, 2)}\n`, 0o600);
  }
}

try {
  main();
} catch (error) {
  console.error(`OOMU RELEASE DATABASE SANITIZER ERROR: ${error.message}`);
  process.exit(1);
}
