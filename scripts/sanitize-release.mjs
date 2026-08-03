#!/usr/bin/env node

import {
  existsSync,
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
  unlinkSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, join, relative, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import process from "node:process";
import { atomicWriteFile } from "./release-manifest.mjs";

const repoRoot = resolve(import.meta.dirname, "..");
const defaultDir = resolve(repoRoot, "release", "pre_alpha");
const policyPath = resolve(repoRoot, "release", "pre_alpha", "sanitizer_manifest.json");

function usage() {
  console.log(`OOMU RELEASE SANITIZER

Usage:
  node scripts/sanitize-release.mjs [--dir <path>] [--execute|--write]
    [--build-id <id> --evidence <path>]

Options:
  --dir <path>   Staging directory to scan. Defaults to release/pre_alpha.
  --execute      Delete matched files after all safety checks pass.
  --write        Alias for --execute.
  --build-id     Build identifier recorded in executed sanitizer evidence.
  --evidence     Write the executed sanitizer result to this path.
  --help         Show this help text.

The sanitizer defaults to dry-run mode and prints every proposed deletion.`);
}

function parseArgs(argv) {
  const options = {
    dir: defaultDir,
    execute: false,
    buildId: null,
    evidence: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") {
      usage();
      process.exit(0);
    }
    if (arg === "--execute" || arg === "--write") {
      options.execute = true;
      continue;
    }
    if (arg === "--dir") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        throw new Error("--dir requires a path.");
      }
      options.dir = resolve(value);
      index += 1;
      continue;
    }
    if (arg.startsWith("--dir=")) {
      options.dir = resolve(arg.slice("--dir=".length));
      continue;
    }
    if (arg === "--build-id" || arg === "--evidence") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value.`);
      options[arg === "--build-id" ? "buildId" : "evidence"] =
        arg === "--evidence" ? resolve(value) : value;
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  return options;
}

function loadPolicy() {
  const policyBytes = readFileSync(policyPath);
  const policy = JSON.parse(policyBytes.toString("utf8"));
  if (
    policy.schema_version !== 3 ||
    policy.kind !== "oomu.release-sanitizer-policy" ||
    !Array.isArray(policy.rules) ||
    policy.rules.length === 0 ||
    !Array.isArray(policy.content_signatures) ||
    policy.content_signatures.length === 0
  ) {
    throw new Error("Release sanitizer policy is missing or unsupported.");
  }
  const identifiers = new Set();
  for (const rule of policy.rules) {
    if (!rule.id || identifiers.has(rule.id)) throw new Error("Sanitizer rule IDs must be unique.");
    if (
      !["file_name_exact", "file_name_prefix", "file_name_suffix"].includes(rule.match?.kind) ||
      typeof rule.match.value !== "string" ||
      rule.match.value.length === 0
    ) {
      throw new Error(`Sanitizer rule ${rule.id} has unsupported path semantics.`);
    }
    identifiers.add(rule.id);
  }
  for (const signature of policy.content_signatures) {
    if (
      !signature.id ||
      identifiers.has(signature.id) ||
      typeof signature.value !== "string" ||
      signature.value.length === 0
    ) {
      throw new Error("Sanitizer content signatures must have unique IDs and non-empty values.");
    }
    identifiers.add(signature.id);
  }
  return {
    ...policy,
    policy_sha256: createHash("sha256").update(policyBytes).digest("hex"),
  };
}

function isInside(root, candidate) {
  const rootPath = existsSync(root) ? realpathSync(root) : resolve(root);
  const candidatePath = existsSync(candidate) ? realpathSync(candidate) : resolve(candidate);
  const pathBetween = relative(rootPath, candidatePath);
  return (
    pathBetween === "" ||
    (!pathBetween.startsWith("..") && !pathBetween.startsWith(sep))
  );
}

function pathSegments(pathValue) {
  return pathValue.split(/[\\/]+/).filter(Boolean);
}

function isOomuTempPath(candidate) {
  const tempRoots = [resolve(tmpdir()), "/tmp", "/private/tmp"].map((entry) =>
    resolve(entry)
  );
  return tempRoots.some((root) => {
    if (!isInside(root, candidate)) {
      return false;
    }
    return pathSegments(relative(root, candidate)).some((segment) =>
      segment.toLowerCase().startsWith("oomu-")
    );
  });
}

function isGeneratedTargetDirectory(candidate) {
  const generatedRepoRoots = [
    resolve(repoRoot, "release"),
    resolve(repoRoot, "out"),
    resolve(repoRoot, "src-tauri", "target"),
  ];

  return (
    generatedRepoRoots.some((root) => isInside(root, candidate)) ||
    isOomuTempPath(candidate)
  );
}

function matchReason(filePath, policy) {
  const name = basename(filePath);
  const lower = name.toLowerCase();
  for (const rule of policy.rules) {
    const value = rule.match.value.toLowerCase();
    const matched =
      (rule.match.kind === "file_name_exact" && lower === value) ||
      (rule.match.kind === "file_name_prefix" && lower.startsWith(value)) ||
      (rule.match.kind === "file_name_suffix" && lower.endsWith(value));
    if (matched) return `${rule.id}: ${rule.description}`;
  }
  return null;
}

function enumerateMatches(targetDir, policy) {
  const matches = [];
  const prohibitedSymbolicLinks = [];
  let filesScanned = 0;
  let contentBytesScanned = 0;
  let symbolicLinksSkipped = 0;
  const prohibitedContentMatches = [];

  function walk(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const entryPath = join(dir, entry.name);
      if (entry.isSymbolicLink()) {
        symbolicLinksSkipped += 1;
        const reason = matchReason(entryPath, policy);
        if (reason) prohibitedSymbolicLinks.push({ path: entryPath, reason });
        console.log(`[SKIPPED] ${entryPath} - symbolic links are never traversed`);
        continue;
      }
      if (entry.isDirectory()) {
        walk(entryPath);
        continue;
      }
      if (!entry.isFile()) {
        continue;
      }
      filesScanned += 1;
      const reason = matchReason(entryPath, policy);
      if (reason) {
        matches.push({ path: entryPath, reason });
      }
      const bytes = readFileSync(entryPath);
      contentBytesScanned += bytes.length;
      for (const signature of policy.content_signatures) {
        if (bytes.indexOf(Buffer.from(signature.value, "utf8")) !== -1) {
          prohibitedContentMatches.push({ path: entryPath, signature_id: signature.id });
        }
      }
    }
  }

  walk(targetDir);
  return {
    matches,
    prohibitedSymbolicLinks,
    prohibitedContentMatches,
    filesScanned,
    contentBytesScanned,
    symbolicLinksSkipped,
  };
}

function ensureSafeTarget(targetDir) {
  if (!existsSync(targetDir)) {
    return { ok: false, reason: "directory does not exist" };
  }

  const stats = lstatSync(targetDir);
  if (stats.isSymbolicLink()) {
    return { ok: false, reason: "target directory is a symbolic link" };
  }
  if (!stats.isDirectory()) {
    return { ok: false, reason: "target path is not a directory" };
  }

  const realTarget = realpathSync(targetDir);
  if (!isGeneratedTargetDirectory(realTarget)) {
    return {
      ok: false,
      reason:
        "target must be under release, out, src-tauri/target, or an OOMU temp staging folder",
    };
  }

  return { ok: true, realTarget };
}

function main() {
  let options;
  let policy;
  try {
    options = parseArgs(process.argv.slice(2));
    policy = loadPolicy();
    if ((options.evidence || options.buildId) && (!options.evidence || !options.buildId)) {
      throw new Error("--evidence and --build-id must be provided together.");
    }
  } catch (error) {
    console.error(`OOMU RELEASE SANITIZER ERROR: ${error.message}`);
    usage();
    process.exit(2);
  }

  const mode = options.execute ? "EXECUTE" : "DRY-RUN (SAFE)";
  const targetDir = resolve(options.dir);
  console.log(`OOMU RELEASE SANITIZER [MODE: ${mode}]`);
  console.log(`Target directory: ${targetDir}`);

  const safety = ensureSafeTarget(targetDir);
  if (!safety.ok) {
    console.log(`No files processed: ${safety.reason}.`);
    process.exit(options.execute ? 1 : 0);
  }

  const scan = enumerateMatches(safety.realTarget, policy);
  if (scan.prohibitedSymbolicLinks.length > 0) {
    throw new Error(
      `Prohibited sanitizer path is a symbolic link: ${scan.prohibitedSymbolicLinks[0].path}`,
    );
  }
  const { matches } = scan;
  for (const match of scan.prohibitedContentMatches) {
    console.log(`[PROHIBITED CONTENT] ${match.path} - ${match.signature_id}`);
  }
  if (matches.length === 0) {
    console.log("No matching release sanitizer targets found.");
  } else {
    for (const match of matches) {
      if (!isInside(safety.realTarget, realpathSync(match.path))) {
        throw new Error(`${match.path} resolved outside the sanitizer target.`);
      }

      if (!options.execute) {
        console.log(`[PROPOSED FOR DELETION] ${match.path} - ${match.reason}`);
        continue;
      }

      unlinkSync(match.path);
      console.log(`[DELETED] ${match.path} - ${match.reason}`);
    }
  }

  console.log(
    `${options.execute ? "Deleted" : "Proposed"} ${matches.length} matched file${
      matches.length === 1 ? "" : "s"
    }.`
  );

  if (options.execute) {
    const verification = enumerateMatches(safety.realTarget, policy);
    if (verification.matches.length > 0 || verification.prohibitedSymbolicLinks.length > 0) {
      throw new Error("Sanitizer verification found prohibited files after execution.");
    }
    if (verification.prohibitedContentMatches.length > 0) {
      const match = verification.prohibitedContentMatches[0];
      throw new Error(
        `Sanitizer verification found prohibited fixture signature ${match.signature_id} in ${match.path}.`,
      );
    }
    if (options.evidence) {
      const report = {
        schema_version: 1,
        kind: "oomu.release-sanitizer-execution",
        status: "passed",
        synthetic: false,
        build_identifier: options.buildId,
        completed_at: new Date().toISOString(),
        target: safety.realTarget,
        policy_sha256: policy.policy_sha256,
        rule_ids: policy.rules.map((rule) => rule.id),
        content_signature_ids: policy.content_signatures.map((signature) => signature.id),
        files_scanned: scan.filesScanned,
        content_bytes_scanned: scan.contentBytesScanned,
        symbolic_links_skipped: scan.symbolicLinksSkipped,
        removed_items: matches.map((match) => ({
          path: relative(safety.realTarget, match.path).split(sep).join("/"),
          reason: match.reason,
        })),
        remaining_prohibited_items: verification.matches.length,
        remaining_prohibited_content_signatures:
          verification.prohibitedContentMatches.length,
      };
      atomicWriteFile(options.evidence, `${JSON.stringify(report, null, 2)}\n`, 0o600);
    }
  }
}

try {
  main();
} catch (error) {
  console.error(`OOMU RELEASE SANITIZER ERROR: ${error.message}`);
  process.exit(1);
}
