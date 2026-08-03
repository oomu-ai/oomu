#!/usr/bin/env node

import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const terminalStates = new Set(["completed", "failed", "blocked", "cancelled"]);
const recoveryScenarios = ["restart", "sleep_wake", "lost_network", "revoked_permissions", "expired_credentials", "denied_approval", "model_failure_fallback", "corrupted_artifact", "channel_delivery_failure"];
const heroModes = ["on_demand", "routine", "restart", "authorized_channel"];

function load(file) { return JSON.parse(readFileSync(file, "utf8")); }
function sameBuild(record, expected) { return record.buildIdentifier === expected.buildIdentifier && record.sourceRevision === expected.sourceRevision && record.artifactDigest === expected.artifactDigest; }

export function validateGoldenTaskMatrix(record, expected) {
  const failures = [];
  if (record?.schemaVersion !== 1 || !sameBuild(record, expected)) failures.push("golden task matrix is not bound to the exact build");
  if (!Array.isArray(record?.runs) || record.runs.length !== 10) failures.push("exactly ten executed golden task runs are required");
  const effects = new Set(); let completed = 0;
  for (const run of record?.runs ?? []) {
    if (run.executed !== true || !terminalStates.has(run.terminalState)) failures.push(`${run.id}: executed terminal state is required`);
    if (run.terminalState === "completed") completed += 1;
    if (!Array.isArray(run.evidence) || run.evidence.length === 0) failures.push(`${run.id}: raw evidence is required`);
    if (run.terminalState !== "completed" && (!run.errorCode || !run.accurateTerminalState)) failures.push(`${run.id}: failures must report an accurate terminal state`);
    for (const effect of run.confirmedEffects ?? []) { if (effects.has(effect.idempotencyKey)) failures.push(`${run.id}: confirmed effect was duplicated`); effects.add(effect.idempotencyKey); if (!effect.verifiedPostcondition) failures.push(`${run.id}: confirmed effect lacks a verified postcondition`); }
  }
  if (completed < 8) failures.push("at least eight golden tasks must complete unattended");
  return { status: failures.length ? "failed" : "passed", completed, failures };
}

export function validateRecoveryMatrix(record, expected) {
  const failures = [];
  if (record?.schemaVersion !== 1 || !sameBuild(record, expected)) failures.push("recovery matrix is not bound to the exact build");
  const scenarios = new Map((record?.scenarios ?? []).map((item) => [item.id, item]));
  for (const id of recoveryScenarios) { const item = scenarios.get(id); if (!item?.executed || !terminalStates.has(item.terminalState) || !item.evidenceArchived) failures.push(`${id}: executed recovery evidence is required`); if ((item?.duplicateConfirmedEffects ?? 0) !== 0) failures.push(`${id}: retry duplicated a confirmed effect`); }
  return { status: failures.length ? "failed" : "passed", failures };
}

export function validateHeroWorkflow(record, expected) {
  const failures = [];
  if (record?.schemaVersion !== 1 || !sameBuild(record, expected)) failures.push("hero workflow evidence is not bound to the exact build");
  const modes = new Map((record?.runs ?? []).map((item) => [item.mode, item]));
  for (const mode of heroModes) { const run = modes.get(mode); if (!run?.executed || run.terminalState !== "completed") failures.push(`${mode}: completed executed hero run is required`); if (!run?.taskEvidenceArchived || !run?.sourceLinksVerified) failures.push(`${mode}: Task evidence and source links are required`); if (!run?.artifact?.docxStructurallyVerified || !run?.artifact?.pdfVisuallyVerified || !run?.artifact?.signedManifestVerified) failures.push(`${mode}: verified DOCX/PDF and signed manifest are required`); }
  if (!record?.cleanMac?.installedSignedNotarizedBuild || record.cleanMac.developerToolingUsed || record.cleanMac.timeToUsefulOutputMs > 10 * 60 * 1000) failures.push("clean Mac hero output must finish within ten minutes without developer tooling");
  return { status: failures.length ? "failed" : "passed", failures };
}

export function validatePrivacyDeclarations(record, expected) {
  const failures = [];
  if (record?.schemaVersion !== 1 || !sameBuild(record, expected)) failures.push("privacy declarations are not bound to the exact build");
  for (const field of ["backgroundHelperDisclosureMatches", "networkBehaviorMatches", "privacyCopyMatches", "entitlementsMatch", "appStoreDeclarationsMatch"]) if (record?.[field] !== true) failures.push(`${field} must be independently confirmed`);
  return { status: failures.length ? "failed" : "passed", failures };
}

export function validateAcceptanceDirectory(directory, expected) {
  const results = {
    golden: validateGoldenTaskMatrix(load(path.join(directory, "golden-task-runs.json")), expected),
    recovery: validateRecoveryMatrix(load(path.join(directory, "recovery-matrix.json")), expected),
    hero: validateHeroWorkflow(load(path.join(directory, "hero-workflow-runs.json")), expected),
    privacy: validatePrivacyDeclarations(load(path.join(directory, "privacy-declarations.json")), expected),
  };
  const failures = Object.entries(results).flatMap(([name, result]) => result.failures.map((failure) => `${name}: ${failure}`));
  return { schemaVersion: 1, status: failures.length ? "failed" : "passed", synthetic: false, results, failures };
}

function main() { const args = Object.fromEntries(process.argv.slice(2).map((item) => item.split("=", 2))); const directory = args["--evidence-dir"]; const expected = { buildIdentifier: args["--build-id"], sourceRevision: args["--source-revision"], artifactDigest: args["--artifact-digest"] }; if (!directory || Object.values(expected).some((value) => !value)) throw new Error("--evidence-dir, --build-id, --source-revision, and --artifact-digest are required"); const result = validateAcceptanceDirectory(directory, expected); console.log(JSON.stringify(result, null, 2)); if (result.status !== "passed") process.exit(1); }
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) { try { main(); } catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exit(1); } }
