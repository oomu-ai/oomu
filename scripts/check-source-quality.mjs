#!/usr/bin/env node

import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { SOURCE_LIMITS, measureSource, metricKeys } from "./source-quality/source-metrics.mjs";

const root = path.resolve(import.meta.dirname, "..");
const lineBaselinePath = process.env.OOMU_SOURCE_LINE_BASELINE
  ? path.resolve(process.env.OOMU_SOURCE_LINE_BASELINE)
  : path.join(root, "scripts", "source-line-baselines.tsv");
const metricBaselinePath = process.env.OOMU_SOURCE_METRIC_BASELINE
  ? path.resolve(process.env.OOMU_SOURCE_METRIC_BASELINE)
  : path.join(root, "scripts", "source-metric-baselines.json");

function walk(directory, predicate) {
  if (!existsSync(directory)) return [];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (["target", "node_modules", ".next", "generated"].includes(entry.name)) continue;
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...walk(absolute, predicate));
    else if (entry.isFile() && predicate(absolute)) files.push(absolute);
  }
  return files;
}

function sourceFiles() {
  return [
    ...walk(path.join(root, "src"), (file) => /\.(?:ts|tsx|mts)$/.test(file)),
    ...walk(path.join(root, "src-tauri", "src"), (file) => file.endsWith(".rs")),
    ...walk(path.join(root, "scripts"), (file) => /\.(?:mjs|js|mts|sh)$/.test(file)),
  ].sort();
}

function readLineBaselines() {
  const baselines = new Map();
  let previousPath = "";
  for (const line of readFileSync(lineBaselinePath, "utf8").split("\n")) {
    if (!line || line.startsWith("#")) continue;
    const [relativePath, maximumText, owner] = line.split("\t");
    const maximum = Number(maximumText);
    if (!relativePath || !Number.isInteger(maximum) || maximum < 1 || !owner) {
      throw new Error(`source-line-ratchet: invalid baseline entry ${line}`);
    }
    if (previousPath && relativePath < previousPath) {
      throw new Error(`source-line-ratchet: baseline must be sorted (${relativePath} follows ${previousPath})`);
    }
    if (baselines.has(relativePath)) {
      throw new Error(`source-line-ratchet: duplicate baseline for ${relativePath}`);
    }
    baselines.set(relativePath, { maximum, owner });
    previousPath = relativePath;
  }
  return baselines;
}

function readMetricBaselines() {
  const document = JSON.parse(readFileSync(metricBaselinePath, "utf8"));
  if (document.version !== 1 || !document.files || typeof document.files !== "object") {
    throw new Error("source-metric-ratchet: baseline must be a version 1 files object");
  }
  const ordered = Object.keys(document.files);
  if (ordered.join("\n") !== [...ordered].sort().join("\n")) {
    throw new Error("source-metric-ratchet: baseline paths must be sorted");
  }
  const allowedKeys = new Set(["owner", ...metricKeys().filter((metric) => metric !== "lines")]);
  for (const [relativePath, entry] of Object.entries(document.files)) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error(`source-metric-ratchet: ${relativePath} must be an object`);
    }
    const unknownKeys = Object.keys(entry).filter((key) => !allowedKeys.has(key));
    if (unknownKeys.length > 0) {
      throw new Error(`source-metric-ratchet: ${relativePath} has unknown keys ${unknownKeys.join(", ")}`);
    }
  }
  return document.files;
}

function metricDetail(measurement, metric) {
  if (metric === "functionLines") return ` (${measurement.functionLinesName})`;
  if (metric === "complexity") return ` (${measurement.complexityName})`;
  return "";
}

function inferredOwner(relativePath, lineBaselines) {
  const reviewed = lineBaselines.get(relativePath)?.owner;
  if (reviewed) return reviewed;
  if (relativePath.startsWith("scripts/")) return "release/quality-gates";
  if (relativePath.startsWith("src-tauri/src/")) {
    const moduleName = relativePath.slice("src-tauri/src/".length).split(/[/.]/)[0];
    return `native/${moduleName}`;
  }
  if (relativePath.startsWith("src/app/components/")) return "renderer/components";
  if (relativePath.startsWith("src/components/")) return "renderer/shell";
  return "renderer/application";
}

function exactMetricBaseline(measurements, lineBaselines) {
  const files = {};
  for (const measurement of measurements) {
    const limits = SOURCE_LIMITS[measurement.kind];
    const entry = { owner: inferredOwner(measurement.relativePath, lineBaselines) };
    for (const metric of metricKeys().filter((candidate) => candidate !== "lines")) {
      if (measurement[metric] > limits[metric]) entry[metric] = measurement[metric];
    }
    if (Object.keys(entry).length > 1) files[measurement.relativePath] = entry;
  }
  return { version: 1, files };
}

export function inspectMeasurement(
  relativePath,
  measurement,
  lineBaselines,
  metricBaselines,
  seenLineBaselines,
  seenMetricBaselines,
) {
  const failures = [];
  const defaults = SOURCE_LIMITS[measurement.kind];
  const lineBaseline = lineBaselines.get(relativePath);
  if (lineBaseline) seenLineBaselines.add(relativePath);
  const lineMaximum = lineBaseline?.maximum ?? defaults.lines;
  if (measurement.lines > lineMaximum) {
    failures.push(`${lineBaseline ? "GROWTH" : "NEW_OVERSIZED"} ${relativePath} has ${measurement.lines} lines (maximum ${lineMaximum})`);
  } else if (lineBaseline && measurement.lines < lineMaximum) {
    failures.push(`HEADROOM ${relativePath} has ${measurement.lines} lines (baseline ${lineMaximum}); lower the baseline to the exact count`);
  }

  const fileBaseline = metricBaselines[relativePath];
  if (fileBaseline) {
    seenMetricBaselines.add(relativePath);
    if (!fileBaseline.owner || typeof fileBaseline.owner !== "string") {
      failures.push(`INVALID_BASELINE ${relativePath} is missing its owner`);
    }
  }
  for (const metric of metricKeys().filter((candidate) => candidate !== "lines")) {
    const baselineMaximum = fileBaseline?.[metric];
    if (baselineMaximum !== undefined && (
      !Number.isInteger(baselineMaximum) || baselineMaximum <= defaults[metric]
    )) {
      failures.push(`UNNECESSARY_BASELINE ${relativePath} ${metric}=${baselineMaximum}`);
      continue;
    }
    const maximum = baselineMaximum ?? defaults[metric];
    const actual = measurement[metric];
    const detail = metricDetail(measurement, metric);
    if (actual > maximum) {
      failures.push(`${baselineMaximum === undefined ? "NEW_EXCESS" : "GROWTH"} ${relativePath} ${metric}=${actual}${detail} (maximum ${maximum})`);
    } else if (baselineMaximum !== undefined && actual < maximum) {
      failures.push(`HEADROOM ${relativePath} ${metric}=${actual}${detail} (baseline ${maximum}); lower the baseline to the exact value`);
    }
  }
  return failures;
}

export function inspectSourceQuality({ files = sourceFiles() } = {}) {
  const lineBaselines = readLineBaselines();
  const metricBaselines = readMetricBaselines();
  const seenLineBaselines = new Set();
  const seenMetricBaselines = new Set();
  const failures = [];
  const measurements = [];

  for (const absolute of files) {
    const relativePath = path.relative(root, absolute).replace(/\\/g, "/");
    const measurement = measureSource(relativePath, readFileSync(absolute, "utf8"));
    measurements.push({ relativePath, ...measurement });
    failures.push(...inspectMeasurement(
      relativePath,
      measurement,
      lineBaselines,
      metricBaselines,
      seenLineBaselines,
      seenMetricBaselines,
    ));
  }

  for (const relativePath of lineBaselines.keys()) {
    if (!seenLineBaselines.has(relativePath)) failures.push(`STALE_BASELINE ${relativePath} no longer exists`);
  }
  for (const relativePath of Object.keys(metricBaselines)) {
    if (!seenMetricBaselines.has(relativePath)) failures.push(`STALE_METRIC_BASELINE ${relativePath} no longer exists`);
  }
  return { failures, measurements, lineBaselines };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = inspectSourceQuality();
  if (process.argv.includes("--print-measurements")) {
    console.log(JSON.stringify(result.measurements, null, 2));
    process.exit(0);
  }
  if (process.argv.includes("--print-baseline")) {
    console.log(JSON.stringify(
      exactMetricBaseline(result.measurements, result.lineBaselines),
      null,
      2,
    ));
    process.exit(0);
  }
  if (result.failures.length > 0) {
    console.error(`source-line-ratchet: failed with ${result.failures.length} violation(s)`);
    for (const failure of result.failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log(`source-line-ratchet: PASS (${result.measurements.length} human-source files; line, byte, physical-line, function, and complexity ceilings verified)`);
}
