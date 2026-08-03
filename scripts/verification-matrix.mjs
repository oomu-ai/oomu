#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const statuses = ["passed", "failed", "environment-blocked", "not-run"];
const environmentBlockPatterns = [
  /Operation not permitted \(os error 1\)/i,
  /permission denied.*(?:bind|socket|process)/i,
  /creating new process[\s\S]{0,240}binding to a port/i,
  /can't assign requested address/i,
  /network is unreachable/i,
];

const checks = [
  ["real-components", "npm", ["run", "check:real-components"]],
  ["source-line-ratchet", "npm", ["run", "check:source-size"]],
  ["p0-architecture", "npm", ["run", "check:p0-architecture"]],
  ["p1-contracts", "npm", ["run", "check:p1-contracts"]],
  ["p1-typescript-schema-parity", "npx", ["vitest", "run", "src/lib/__tests__/p1Contracts.test.ts"]],
  ["p1-rust-schema-parity", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "p1_contracts::tests"]],
  ["novice-first-ui", "npm", ["run", "check:novice-ui"]],
  ["p1-microsoft365-ui", "npx", ["vitest", "run", "src/app/components/integrations/IntegrationsScreen.test.tsx", "src/app/components/integrations/microsoft365/Microsoft365ControlPanel.test.tsx", "src/app/components/__tests__/SetupJourney.test.tsx"]],
  ["p1-documents-ui", "npx", ["vitest", "run", "src/app/components/artifacts/ArtifactStudio.test.tsx", "src/app/components/artifacts/review/CreateDocumentAction.test.tsx", "src/app/components/chat/ShieldApprovalDialog.test.tsx", "src/lib/artifacts/workbooks/schema.test.ts"]],
  ["module-cycles", "npm", ["run", "check:module-cycles"]],
  ["unused-exports", "npm", ["run", "check:unused-exports"]],
  ["repository-hygiene", "npm", ["run", "check:repository-hygiene"]],
  ["bundle-size", "npm", ["run", "check:bundle-size"]],
  ["i18n", "npm", ["run", "check:i18n"]],
  ["typecheck", "npm", ["run", "typecheck"]],
  ["lint", "npm", ["run", "lint"]],
  ["frontend-tests", "npx", ["vitest", "run"]],
  ["static-build", "npm", ["run", "build"], { environmentSensitive: true }],
  ["cargo-check", "cargo", ["check", "--manifest-path", "src-tauri/Cargo.toml"]],
  ["auto-route-real-classifier", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "inference::dynamic_routing::tests::installed_e2b_real_auto_route_corpus", "--", "--ignored", "--exact", "--nocapture"], { qualifiedOnly: true }],
  ["cargo-real-component-tests", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml"], { environmentSensitive: true }],
  ["p0-golden-discovery", "npm", ["run", "eval:p0-golden:discovery"]],
  ["bundle-size-final", "npm", ["run", "check:bundle-size"]],
];

export function classifyVerificationResult({ profile, result, environmentSensitive = false }) {
  if (result.error?.code === "ENOENT") return "not-run";
  if (result.status === 0) return "passed";
  const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  if (
    profile === "local" &&
    environmentSensitive &&
    environmentBlockPatterns.some((pattern) => pattern.test(output))
  ) {
    return "environment-blocked";
  }
  return "failed";
}

function lastMeaningfulLines(output, maximum = 12) {
  return output
    .split("\n")
    .map((line) => line.trimEnd())
    .filter(Boolean)
    .slice(-maximum)
    .join("\n");
}

export function runVerificationMatrix(profile) {
  if (!(["local", "qualified"].includes(profile))) throw new Error(`unknown verification profile: ${profile}`);
  const records = [];
  for (const [name, executable, args, options = {}] of checks) {
    if (options.qualifiedOnly && profile !== "qualified") continue;
    const started = Date.now();
    const result = spawnSync(executable, args, {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        OOMU_VERIFICATION_PROFILE: profile,
        OOMU_QUALIFIED_RUNNER: profile === "qualified" ? "1" : "0",
        PYTHONDONTWRITEBYTECODE: "1",
      },
      maxBuffer: 100 * 1024 * 1024,
      timeout: 15 * 60 * 1000,
    });
    const status = classifyVerificationResult({ profile, result, ...options });
    const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
    const record = {
      name,
      status,
      elapsedMs: Date.now() - started,
      exitCode: result.status,
      ...(status === "passed" ? {} : { detail: lastMeaningfulLines(output || result.error?.message || "no output") }),
    };
    records.push(record);
    console.log(`[verification] ${name}: ${status} (${record.elapsedMs} ms)`);
    if (record.detail) console.log(record.detail);
  }

  const counts = Object.fromEntries(statuses.map((status) => [status, 0]));
  for (const record of records) counts[record.status] += 1;
  const status = counts.failed > 0
    ? "failed"
    : counts["environment-blocked"] > 0
      ? "environment-blocked"
      : counts["not-run"] > 0
        ? "not-run"
        : "passed";
  return { schemaVersion: 1, profile, status, counts, checks: records };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const profileArgument = process.argv.find((argument) => argument.startsWith("--profile="));
  const profile = profileArgument?.slice("--profile=".length) ?? "local";
  const report = runVerificationMatrix(profile);
  console.log(JSON.stringify(report, null, 2));
  if (report.status !== "passed") process.exit(1);
}
