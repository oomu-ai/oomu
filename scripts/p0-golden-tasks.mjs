#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import {
  assertNativeCommandHandlerWiring,
  registeredProductionCommands,
} from "./native-command-registry.mjs";

const root = path.resolve(import.meta.dirname, "..");
const definitionsPath = path.join(root, "evaluations", "p0", "golden-tasks.json");
const evidenceClasses = new Set([
  "model_assertion",
  "observed_result",
  "executed_mutation",
  "verified_postcondition",
  "signed_artifact",
]);
const authorityClasses = new Set(["read_only", "shield", "user_grant"]);

export function discoverGoldenTasks() {
  const definitions = JSON.parse(readFileSync(definitionsPath, "utf8"));
  assertNativeCommandHandlerWiring(root);
  const commands = registeredProductionCommands(root);
  const failures = [];
  const seenIds = new Set();
  if (definitions.schemaVersion !== 1) failures.push("unsupported golden-task schema version");
  if (!Array.isArray(definitions.tasks) || definitions.tasks.length !== 10) {
    failures.push("exactly ten golden tasks are required");
  }
  const prohibitedText = /\b(mock|fixture|canned|expectedOutput|bypass(?:es|ed)?Shield|alternateRuntime)\b/i;
  for (const task of definitions.tasks ?? []) {
    if (!/^p0-[0-9]{2}-[a-z0-9-]+$/.test(task.id) || seenIds.has(task.id)) {
      failures.push(`invalid or duplicate task id: ${task.id}`);
    }
    seenIds.add(task.id);
    if (typeof task.outcome !== "string" || task.outcome.length < 12) {
      failures.push(`${task.id}: observable outcome is required`);
    }
    if (prohibitedText.test(JSON.stringify(task))) {
      failures.push(`${task.id}: prohibited substitute execution marker`);
    }
    if (!Array.isArray(task.productionCommands) || task.productionCommands.length === 0) {
      failures.push(`${task.id}: production command mapping is required`);
    }
    for (const command of task.productionCommands ?? []) {
      if (!commands.has(command.name)) failures.push(`${task.id}: unregistered command ${command.name}`);
      if (!authorityClasses.has(command.authority)) {
        failures.push(`${task.id}: invalid authority for ${command.name}`);
      }
    }
    if (!Array.isArray(task.expectedPostconditions) || task.expectedPostconditions.length === 0) {
      failures.push(`${task.id}: expected postconditions are required`);
    }
    for (const postcondition of task.expectedPostconditions ?? []) {
      if (!evidenceClasses.has(postcondition.evidenceClass)) {
        failures.push(`${task.id}: invalid evidence class ${postcondition.evidenceClass}`);
      }
      if (typeof postcondition.observation !== "string" || postcondition.observation.length < 12) {
        failures.push(`${task.id}: postcondition observation is incomplete`);
      }
    }
  }
  const requiredMetrics = new Set(definitions.requiredMetrics ?? []);
  for (const metric of [
    "completionStatus",
    "approvalCount",
    "retryCount",
    "elapsedMs",
    "artifactValidity",
    "evidenceCompleteness",
  ]) {
    if (!requiredMetrics.has(metric)) failures.push(`missing required run metric: ${metric}`);
  }
  return { definitions, failures };
}

export function discoveryReport() {
  const { definitions, failures } = discoverGoldenTasks();
  const packageManifest = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8"));
  let sourceRevision = "unavailable";
  try {
    sourceRevision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  } catch {
    failures.push("current source revision is unavailable");
  }
  return {
    schemaVersion: 1,
    mode: "discovery",
    status: failures.length === 0 ? "passed" : "failed",
    discoveredTaskCount: definitions.tasks?.length ?? 0,
    build: { version: packageManifest.version, sourceRevision },
    model: { executionStatus: "not-run", reason: "discovery mode does not invoke a model" },
    machine: { architecture: os.arch(), platform: os.platform(), osRelease: os.release() },
    failures,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.some((argument) => argument.startsWith("--mode=") && argument !== "--mode=discovery")) {
    console.error("p0-golden-tasks: only genuine discovery mode is implemented in Sprint 224");
    process.exit(2);
  }
  const report = discoveryReport();
  console.log(JSON.stringify(report, null, 2));
  if (report.status !== "passed") process.exit(1);
}
