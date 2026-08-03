#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");

const forbidden = [
  {
    pattern: /(^|\/)(?:target|dist|build|coverage|\.next|node_modules|\.cache|\.turbo)(\/|$)/,
    reason: "tracked build or cache output",
  },
  { pattern: /(^|\/)\.DS_Store$/, reason: "OS metadata" },
  { pattern: /(^|\/).*\.log$/i, reason: "mutable log" },
  { pattern: /(^|\/)__pycache__(\/|$)/, reason: "Python cache" },
  { pattern: /\.py[co]$/i, reason: "compiled Python bytecode" },
  { pattern: /^sandbox\//, reason: "orphan sandbox seed" },
  {
    pattern: /^src-tauri\/resources\/python\/resource-placeholder\.txt$/,
    reason: "portable-runtime marker",
  },
  {
    pattern: /^public\/(?:file|globe|next|vercel|window)\.svg$/,
    reason: "default Next.js asset",
  },
  { pattern: /^public\/oomu-raven\.png$/, reason: "duplicate Raven raster" },
];

export function repositoryPathFailure(file) {
  const match = forbidden.find(({ pattern }) => pattern.test(file));
  return match ? `${file}: ${match.reason}` : null;
}

export function inspectRepositoryHygiene(repoRoot, tracked) {
  const failures = [];
  for (const file of tracked) {
    const pathFailure = repositoryPathFailure(file);
    if (pathFailure) failures.push(pathFailure);
  }
  const assetFiles = tracked.filter(
    (file) =>
      existsSync(path.join(repoRoot, file)) &&
      /^public\/.+\.(?:png|jpe?g|gif|webp|svg|ico)$/i.test(file),
  );
  const digestOwners = new Map();
  for (const file of assetFiles) {
    const absolute = path.join(repoRoot, file);
    if (!existsSync(absolute) || !statSync(absolute).isFile()) continue;
    const digest = createHash("sha256").update(readFileSync(absolute)).digest("hex");
    const previous = digestOwners.get(digest);
    if (previous) {
      failures.push(`${file}: byte-for-byte duplicate of ${previous}`);
    } else {
      digestOwners.set(digest, file);
    }
  }
  return { assetFiles, failures };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const tracked = execFileSync("git", ["ls-files", "-z"], {
    cwd: root,
    encoding: "utf8",
  }).split("\0").filter(Boolean);
  const { assetFiles, failures } = inspectRepositoryHygiene(root, tracked);
  if (failures.length > 0) {
    console.error("repository-hygiene: FAIL");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }

  console.log(
    `repository-hygiene: PASS (${tracked.length} tracked files; ${assetFiles.length} reviewed public assets)`,
  );
}
