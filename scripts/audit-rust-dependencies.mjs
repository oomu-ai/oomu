import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const root = resolve(import.meta.dirname, "..");
const lockfile = "src-tauri/Cargo.lock";
const manifest = "src-tauri/Cargo.toml";
const canonicalTarget = "aarch64-apple-darwin";
const PDF_ADVISORY = "RUSTSEC-2026-0187";
const reviewedUnreachableFindings = new Set([
  "RUSTSEC-2026-0194:quick-xml:0.39.2",
  "RUSTSEC-2026-0195:quick-xml:0.39.2",
]);

function runCargo(args, acceptedStatuses = new Set([0])) {
  const result = spawnSync("cargo", args, {
    cwd: root,
    env: process.env,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (!acceptedStatuses.has(result.status ?? -1)) {
    throw new Error((result.stderr || result.stdout || "cargo failed").trim());
  }
  return result;
}

function findingKey(finding) {
  return [
    finding?.advisory?.id,
    finding?.package?.name,
    finding?.package?.version,
  ].join(":");
}

export function classifyRustAdvisoryReport(report, targetTreeOutput) {
  const settingsIgnores = report?.settings?.ignore ?? [];
  if (settingsIgnores.length !== 0) {
    throw new Error("Rust advisory scan must not contain ignore entries.");
  }
  const findings = report?.vulnerabilities?.list ?? [];
  if (findings.some((finding) => finding?.advisory?.id === PDF_ADVISORY)) {
    throw new Error(`${PDF_ADVISORY} remains in the release lockfile.`);
  }
  const unreviewed = findings.filter(
    (finding) => !reviewedUnreachableFindings.has(findingKey(finding)),
  );
  if (unreviewed.length !== 0) {
    throw new Error(
      `Unreviewed Rust advisories remain: ${unreviewed.map(findingKey).join(", ")}`,
    );
  }
  if (findings.length !== 0 && targetTreeOutput.trim() !== "") {
    throw new Error(
      "The reviewed quick-xml finding became reachable in the canonical macOS target graph.",
    );
  }
  return {
    status: "passed",
    full_lockfile_scanned: true,
    ignore_entries: [],
    canonical_target: canonicalTarget,
    reachable_vulnerability_count: 0,
    full_lockfile_finding_count: findings.length,
    target_unreachable_findings: findings.map((finding) => ({
      advisory_id: finding.advisory.id,
      package: finding.package.name,
      version: finding.package.version,
      reason:
        "wayland-scanner is a Linux-only build dependency absent from the canonical macOS arm64 graph",
    })),
    pdf_advisory: {
      id: PDF_ADVISORY,
      present: false,
      ignored: false,
    },
  };
}

export function parseCargoAuditReport(stdout, stderr) {
  const auditOutput = [stdout, stderr]
    .map((output) => output.trim())
    .find((output) => output.includes('{"database"'));
  if (!auditOutput) {
    throw new Error("cargo audit did not return its required JSON report.");
  }
  return JSON.parse(auditOutput.slice(auditOutput.indexOf('{"database"')));
}

export function runRustAdvisoryGate({ noFetch = false } = {}) {
  const auditArgs = ["audit", "--file", lockfile, "--json"];
  if (noFetch) auditArgs.push("--no-fetch");
  const audit = runCargo(auditArgs, new Set([0, 1]));
  const report = parseCargoAuditReport(audit.stdout, audit.stderr);
  const targetTree = runCargo([
    "tree",
    "--manifest-path",
    manifest,
    "--target",
    canonicalTarget,
    "-i",
    "quick-xml@0.39.2",
  ]);
  const verdict = classifyRustAdvisoryReport(report, targetTree.stdout);
  const lopdf = /\[\[package\]\]\s+name = "lopdf"\s+version = "([^"]+)"/u.exec(
    readFileSync(resolve(root, lockfile), "utf8"),
  )?.[1];
  if (lopdf !== "0.42.0") {
    throw new Error(`Expected lopdf 0.42.0 in the release lockfile; found ${lopdf ?? "none"}.`);
  }
  return {
    ...verdict,
    parser: { name: "lopdf", version: lopdf },
    advisory_database: report.database,
    dependency_count: report.lockfile?.["dependency-count"],
    raw_full_scan: report,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const result = runRustAdvisoryGate({ noFetch: process.argv.includes("--no-fetch") });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}
