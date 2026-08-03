#!/usr/bin/env node

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  rustCrateDependencies,
  typeScriptImportSpecifiers,
} from "./architecture/source-imports.mjs";

const root = path.resolve(import.meta.dirname, "..");
const baselinePath = process.env.OOMU_MODULE_CYCLE_BASELINE
  ? path.resolve(process.env.OOMU_MODULE_CYCLE_BASELINE)
  : path.join(root, "scripts", "module-cycle-baseline.json");

const forbiddenEdges = [
  ["rust:sovereign_identity", "rust:db", "native authority must not depend on persistence"],
  ["rust:sovereign_identity", "rust:inference", "native authority must not depend on inference"],
  ["rust:sovereign_identity", "rust:workflow_runtime", "native authority must not depend on workflow orchestration"],
  ["rust:sovereign_identity", "rust:mcp", "native authority must not depend on MCP"],
  ["rust:sovereign_identity", "rust:tools", "native authority must not depend on tools"],
  ["rust:db", "rust:workflow_runtime", "persistence must not depend on workflow orchestration"],
  ["rust:db", "rust:mcp", "persistence must not depend on MCP"],
  ["rust:inference", "rust:workflow_runtime", "inference must not depend on workflow orchestration"],
  ["rust:tools", "rust:workflow_runtime", "tools must not depend on workflow orchestration"],
  ["rust:tools", "rust:mcp", "tools must use the neutral native application port"],
  ["ts:src/components/Sidebar", "ts:src/components/AppShell", "leaf UI must use the neutral navigation contract"],
];

function walk(directory, predicate) {
  const output = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === "target" || entry.name === "node_modules" || entry.name === ".next") continue;
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) output.push(...walk(absolute, predicate));
    else if (predicate(absolute)) output.push(absolute);
  }
  return output;
}

function normalizeTsModule(file) {
  return path.relative(root, file).replace(/\\/g, "/").replace(/\.(?:tsx?|mts)$/, "");
}

function resolveTsImport(sourceFile, specifier, known) {
  let candidate;
  if (specifier.startsWith("@/")) candidate = path.join(root, "src", specifier.slice(2));
  else if (specifier.startsWith(".")) candidate = path.resolve(path.dirname(sourceFile), specifier);
  else return null;
  const normalized = normalizeTsModule(candidate);
  for (const option of [normalized, `${normalized}/index`]) {
    if (known.has(option)) return `ts:${option}`;
  }
  return null;
}

const graph = new Map();
const addNode = (node) => {
  if (!graph.has(node)) graph.set(node, new Set());
};
const addEdge = (from, to) => {
  addNode(from);
  addNode(to);
  if (from !== to) graph.get(from).add(to);
};

const ownershipPaths = ["p0-domain-ownership.json", "p1-domain-ownership.json"];
for (const ownershipFile of ownershipPaths) {
  const ownership = JSON.parse(readFileSync(path.join(root, "scripts", ownershipFile), "utf8"));
  for (const domain of ownership.domains ?? []) {
    for (const node of domain.cycleNodes ?? []) addNode(node);
  }
}

const tsFiles = walk(path.join(root, "src"), (file) => /\.(?:ts|tsx|mts)$/.test(file));
const tsKnown = new Set(tsFiles.map(normalizeTsModule));
for (const file of tsFiles) {
  const from = `ts:${normalizeTsModule(file)}`;
  addNode(from);
  const source = readFileSync(file, "utf8");
  for (const specifier of typeScriptImportSpecifiers(file, source)) {
    const target = resolveTsImport(file, specifier, tsKnown);
    if (target) addEdge(from, target);
  }
}

const rustFiles = walk(path.join(root, "src-tauri", "src"), (file) => file.endsWith(".rs"));
for (const file of rustFiles) {
  const relative = path.relative(path.join(root, "src-tauri", "src"), file).replace(/\\/g, "/");
  const fromModule = relative.startsWith("bin/")
    ? `bin_${path.basename(relative, ".rs")}`
    : relative.split("/")[0].replace(/\.rs$/, "");
  const from = `rust:${fromModule}`;
  addNode(from);
  const source = readFileSync(file, "utf8");
  for (const dependency of rustCrateDependencies(source)) {
    addEdge(from, `rust:${dependency}`);
  }
}

let index = 0;
const stack = [];
const onStack = new Set();
const indexes = new Map();
const lowLinks = new Map();
const cycles = [];

function visit(node) {
  indexes.set(node, index);
  lowLinks.set(node, index);
  index += 1;
  stack.push(node);
  onStack.add(node);

  for (const target of graph.get(node) ?? []) {
    if (!indexes.has(target)) {
      visit(target);
      lowLinks.set(node, Math.min(lowLinks.get(node), lowLinks.get(target)));
    } else if (onStack.has(target)) {
      lowLinks.set(node, Math.min(lowLinks.get(node), indexes.get(target)));
    }
  }

  if (lowLinks.get(node) !== indexes.get(node)) return;
  const component = [];
  let member;
  do {
    member = stack.pop();
    onStack.delete(member);
    component.push(member);
  } while (member !== node);
  if (component.length > 1) cycles.push(component.sort());
}

for (const node of [...graph.keys()].sort()) {
  if (!indexes.has(node)) visit(node);
}

const keys = cycles.map((cycle) => cycle.join("|")).sort();
if (process.argv.includes("--print-baseline")) {
  console.log(JSON.stringify({ cycles: keys }, null, 2));
  process.exit(0);
}

const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
const approved = new Set(baseline.cycles ?? []);
const current = new Set(keys);
const introduced = keys.filter((key) => !approved.has(key));
const stale = [...approved].filter((key) => !current.has(key));
const forbidden = forbiddenEdges.filter(([from, to]) => graph.get(from)?.has(to));

if (introduced.length || stale.length || forbidden.length) {
  console.error("module-cycle-ratchet: FAIL");
  for (const key of introduced) console.error(`  - NEW_CYCLE ${key}`);
  for (const key of stale) console.error(`  - STALE_BASELINE ${key}`);
  for (const [from, to, reason] of forbidden) {
    console.error(`  - FORBIDDEN_EDGE ${from} -> ${to}: ${reason}`);
  }
  process.exit(1);
}

console.log(`module-cycle-ratchet: PASS (${graph.size} modules, ${keys.length} reviewed existing cycles)`);
