#!/usr/bin/env node

import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const contractPath = path.join(root, "scripts", "p0-domain-ownership.json");
const sourceBaselinePath = path.join(root, "scripts", "source-line-baselines.tsv");

function filesUnder(directory) {
  if (!existsSync(directory)) return [];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(absolute));
    else if (entry.isFile() && /\.(?:rs|ts|tsx|mts)$/.test(entry.name)) files.push(absolute);
  }
  return files;
}

export function inspectP0Architecture() {
  const contract = JSON.parse(readFileSync(contractPath, "utf8"));
  const failures = [];
  if (contract.version !== 1) failures.push("unsupported domain ownership contract version");
  if (!Number.isInteger(contract.maximumLinesPerDomainFile) || contract.maximumLinesPerDomainFile < 1) {
    failures.push("maximumLinesPerDomainFile must be a positive integer");
  }

  const names = new Set();
  const roots = new Set();
  const cycleNodes = new Set();
  for (const domain of contract.domains ?? []) {
    if (names.has(domain.name)) failures.push(`duplicate domain: ${domain.name}`);
    names.add(domain.name);
    if (!Number.isInteger(domain.sprint) || domain.sprint < 225 || domain.sprint > 231) {
      failures.push(`${domain.name}: invalid owning sprint`);
    }
    if (!Array.isArray(domain.roots) || domain.roots.length < 2) {
      failures.push(`${domain.name}: native and renderer roots are required`);
    }
    for (const relativeRoot of domain.roots ?? []) {
      if (path.isAbsolute(relativeRoot) || relativeRoot.includes("..") || roots.has(relativeRoot)) {
        failures.push(`${domain.name}: invalid or duplicate root ${relativeRoot}`);
        continue;
      }
      roots.add(relativeRoot);
      for (const file of filesUnder(path.join(root, relativeRoot))) {
        const lineCount = readFileSync(file, "utf8").split("\n").length - 1;
        if (lineCount > contract.maximumLinesPerDomainFile) {
          failures.push(
            `${path.relative(root, file)}: ${lineCount} lines exceeds P0 domain ceiling ${contract.maximumLinesPerDomainFile}`,
          );
        }
      }
    }
    for (const node of domain.cycleNodes ?? []) {
      if (!/^(?:rust|ts):/.test(node) || cycleNodes.has(node)) {
        failures.push(`${domain.name}: invalid or duplicate cycle node ${node}`);
      }
      cycleNodes.add(node);
    }
  }

  const requiredDomains = [
    "projects",
    "tasks",
    "connectors",
    "routines",
    "browser_automation",
    "artifacts",
    "delegation",
  ];
  for (const domain of requiredDomains) {
    if (!names.has(domain)) failures.push(`missing reserved P0 domain: ${domain}`);
  }

  const baselinedSources = new Set(
    readFileSync(sourceBaselinePath, "utf8")
      .split("\n")
      .filter((line) => line && !line.startsWith("#"))
      .map((line) => line.split("\t")[0]),
  );
  for (const integrationFile of contract.thinIntegrationFiles ?? []) {
    if (!baselinedSources.has(integrationFile)) {
      failures.push(`${integrationFile}: thin integration file lacks a non-growth baseline`);
    }
  }
  return failures;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const failures = inspectP0Architecture();
  if (failures.length > 0) {
    console.error("p0-architecture: FAIL");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log("p0-architecture: PASS (7 reserved domains, cycle nodes, and line ceilings verified)");
}
