import { existsSync, lstatSync, readFileSync, readdirSync, statSync } from "node:fs";
import { relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(process.cwd());
const joinToken = (...parts) => parts.join("");

const prohibitedRuntimeMechanisms = [
  [joinToken("simulate", "_failure"), new RegExp(`\\b${joinToken("simulate", "_failure")}\\b`)],
  [joinToken("placeholder", " gateway worker"), new RegExp(`\\bspawn_${joinToken("placeholder", "_worker")}\\b`)],
  [joinToken("manufactured", " workflow compose"), new RegExp(`\\b${joinToken("deterministic", "_compose_fallback")}\\b`)],
  [joinToken("fabricated", " benchmark loop"), new RegExp(`\\b${joinToken("synthetic", "_agent_loop")}\\b`)],
  [joinToken("fabricated", " connection report"), new RegExp(`\\b${joinToken("Connection", "SimulationReport")}\\b`)],
  [joinToken("estimated", " benchmark memory"), new RegExp(`\\b${joinToken("estimate", "_memory_pressure_mb")}\\b`)],
  [joinToken("fixed", " benchmark tokens"), new RegExp(`\\b${joinToken("synthetic", "_tokens")}\\b`)],
  [joinToken("browser", " connectivity inference"), new RegExp(joinToken("navigator", "\\.onLine"))],
  [joinToken("non-probing", " network success"), new RegExp(joinToken("offline-safe", "-local-only"))],
  [joinToken("canned", " mod response"), new RegExp(`\\b${joinToken("pundamentals", "_fallback_pun")}\\b`)],
  [joinToken("browser", " preview state"), new RegExp(joinToken("isSandbox", "PreviewMode"))],
  [joinToken("browser", " preview persistence"), new RegExp(joinToken("oomu:sandbox", "-preview-mode"))],
  [joinToken("fixed", " model readiness"), new RegExp(joinToken("Model weights indexed", " on host filesystem"))],
  [joinToken("manufactured", " authorization success"), new RegExp(joinToken("Hallucinated", "Success|hallucinated_", "success"))],
  [joinToken("estimated", " hardware memory"), new RegExp(`\\b${joinToken("estimated", "_vram_gb")}\\b`)],
  [joinToken("fabricated", " hardware score"), new RegExp(`\\b${joinToken("compute", "_score")}\\b`)],
  [joinToken("project-local", " airlock mount"), new RegExp(joinToken("project_root\\.join\\(\"airlock_", "exports\"\\)"))],
  [joinToken("invented", " workflow file path"), new RegExp(joinToken("workspace/vwa-", "output\\.txt"))],
  [joinToken("invented", " workflow file content"), new RegExp(joinToken("OOMU VWA verified", " file write"), "i")],
  [joinToken("unbound", " remote tool assertion"), new RegExp(joinToken("must immediately call the corresponding", " tool"), "i")],
  [joinToken("false", " conversational continuation"), new RegExp(joinToken("Continuing with conversational", " response"), "i")],
  [joinToken("static", " workflow capability substitution"), new RegExp(`\\b${joinToken("buildStaticWorkflow", "CapabilityCatalog")}\\b`)],
  [joinToken("fallback", " success copy"), new RegExp(`\\b${joinToken("fallback", "Success")}\\b`)],
  [joinToken("fabricated", " diagnostic agent identity"), new RegExp(joinToken("unwrap_or_else\\(\\|\\| \"agent-", "oomu\"\\.to_string\\(\\)\\)"))],
  [joinToken("runtime", " fallback invoke wrapper"), new RegExp(`\\b${joinToken("safe", "Invoke")}\\b`)],
  [joinToken("unverified", " action claim passthrough"), new RegExp(`\\b${joinToken("simulated", "ToolStatement")}\\b`)],
  [
    joinToken("masked", " AppleScript collection failure"),
    new RegExp(
      joinToken("def degraded_collection_", "result\\([\\s\\S]{0,500}?return text_", "result\\("),
    ),
  ],
  [
    joinToken("masked", " AppleScript permission failure"),
    new RegExp(
      joinToken("def permission_blocked_or_timed_out_", "result\\([\\s\\S]{0,300}?return text_", "result\\("),
    ),
  ],
];

const prohibitedWorktreePaths = [
  ["hybrid development bridge", joinToken("scripts/tauri-", "mock/handler.mts")],
  ["hybrid development launcher", "scripts/dev.mjs"],
  ["hard-coded graph generator", joinToken("src/lib/generate_", "oomu_metrics.py")],
  ["portable runtime marker", joinToken("src-tauri/resources/python/resource-", "placeholder.txt")],
];

function worktreeFilesUnder(relativeRoot) {
  const start = resolve(root, relativeRoot);
  if (!existsSync(start)) return [];
  const files = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolute = resolve(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile() || entry.isSymbolicLink()) {
        files.push(relative(root, absolute).split(sep).join("/"));
      }
    }
  };
  visit(start);
  return files;
}

function implementationWorktreeFiles() {
  return ["src", "src-tauri/src", "src-tauri/resources/mcp", "scripts"]
    .flatMap(worktreeFilesUnder)
    .filter(isRuntimeImplementation);
}

function isRuntimeImplementation(path) {
  const normalized = path.split(sep).join("/");
  if (
    /\b(__tests__|testdata|fixtures)\b/.test(normalized) ||
    /(?:^|\.)test\.[^.]+$/.test(normalized) ||
    /(?:^|\/)test_[^/]+\.py$/.test(normalized)
  ) {
    return false;
  }
  return (
    normalized.startsWith("src/") ||
    normalized.startsWith("src-tauri/src/") ||
    normalized.startsWith("src-tauri/resources/mcp/") ||
    normalized.startsWith("scripts/")
  );
}

function readText(path) {
  const bytes = readFileSync(resolve(root, path));
  if (bytes.includes(0)) return null;
  return bytes.toString("utf8");
}

export function inspectRuntimeText(path, source) {
  const failures = [];
  for (const [label, pattern] of prohibitedRuntimeMechanisms) {
    if (pattern.test(source)) failures.push(`${path}: prohibited ${label}`);
  }
  return failures;
}

function validateNativeArtifacts() {
  const failures = [];
  for (const path of worktreeFilesUnder("src-tauri/resources/python")) {
    if (/\/unittest\/mock\.py$/i.test(path)) {
      failures.push(`${path}: test-double framework must not ship in the portable runtime`);
    }
  }
  for (const relative of ["src-tauri/binaries", "src-tauri/resources/python/bin"]) {
    const prefix = `${relative}/`;
    for (const path of worktreeFilesUnder(relative).filter((entry) => entry.startsWith(prefix))) {
      const absolute = resolve(root, path);
      const metadata = lstatSync(absolute);
      if (!metadata.isFile() && !metadata.isSymbolicLink()) continue;
      const target = statSync(absolute);
      if (target.size === 0) failures.push(`${path}: declared executable is empty`);
      if ((target.mode & 0o111) === 0) failures.push(`${path}: declared executable is not executable`);
      const bytes = readFileSync(absolute).subarray(0, 4);
      const knownExecutable =
        (bytes[0] === 0xcf && bytes[1] === 0xfa && bytes[2] === 0xed && bytes[3] === 0xfe) ||
        (bytes[0] === 0x7f && bytes[1] === 0x45 && bytes[2] === 0x4c && bytes[3] === 0x46) ||
        (bytes[0] === 0x4d && bytes[1] === 0x5a) ||
        (bytes[0] === 0x23 && bytes[1] === 0x21);
      if (!knownExecutable) failures.push(`${path}: declared executable has an unknown file type`);
    }
  }
  return failures;
}

function validateArchitectureContracts() {
  const packageManifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
  const dev = packageManifest.scripts?.dev ?? "";
  const releaseEntrypoint = readFileSync(resolve(root, "scripts/release.mjs"), "utf8");
  const appContext = readFileSync(resolve(root, "src/context/AppContext.tsx"), "utf8");
  const network = readFileSync(resolve(root, "src-tauri/src/tools/network.rs"), "utf8");
  const benchmark = readFileSync(
    resolve(root, "tools/developer-tools/src/bin/oomu_bench.rs"),
    "utf8",
  );
  const cargoManifest = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8");
  const developerToolsManifest = readFileSync(
    resolve(root, "tools/developer-tools/Cargo.toml"),
    "utf8",
  );
  const workflowEvaluation = readFileSync(
    resolve(root, "scripts/workflow-compose-eval.mts"),
    "utf8",
  );
  const failures = [];

  if (!/\btauri dev\b/.test(dev) || /tauri-mock|scripts\/dev\.mjs/.test(dev)) {
    failures.push("package.json: development must start the real Tauri backend");
  }
  if (!/["']run["'],\s*["']check:real-components["']/.test(releaseEntrypoint)) {
    failures.push("scripts/release.mjs: real-component gate is missing from the canonical release entrypoint");
  }
  for (const command of ["get_degraded_mode_status", "get_local_model_status", "run_network_diagnostic"]) {
    if (!appContext.includes(command)) failures.push(`AppContext: native health command ${command} is missing`);
  }
  if (!network.includes("UdpSocket::bind") || !network.includes("resolve_destination")) {
    failures.push("network diagnostic: observed local route and policy-bound endpoint probes are required");
  }
  for (const evidenceField of [
    "raw_model_samples",
    "generated_token_count",
    "resident_memory_before_bytes",
    "weights_sha256",
    "MachineProfile",
  ]) {
    if (!benchmark.includes(evidenceField)) {
      failures.push(`oomu_bench: measured evidence field ${evidenceField} is missing`);
    }
  }
  for (const internalName of ["oomu_bench", "ark_verify", "stage_pre_alpha", "debug_db", "debug_executions", "sanitize_release_db"]) {
    if (cargoManifest.includes(`name = "${internalName}"`)) {
      failures.push(`Cargo package: internal utility ${internalName} must not be a Tauri binary target`);
    }
  }
  if (!/name = "oomu-developer-tools"/u.test(developerToolsManifest) ||
      !/name = "oomu_bench"/u.test(developerToolsManifest)) {
    failures.push("oomu_bench: benchmark must remain directly available in the isolated developer-tools package");
  }
  for (const realWorkflowProof of [
    "save_workflow",
    "run_workflow",
    "controlledRuntimeExecuted",
    "if (dryRun)",
    "process.exitCode = 1",
  ]) {
    if (!workflowEvaluation.includes(realWorkflowProof)) {
      failures.push(`workflow evaluation: required real-runtime gate ${realWorkflowProof} is missing`);
    }
  }
  return failures;
}

export function runRealComponentCheck() {
  const failures = [];
  for (const [label, path] of prohibitedWorktreePaths) {
    if (existsSync(resolve(root, path))) failures.push(`${path}: prohibited ${label}`);
  }
  for (const path of implementationWorktreeFiles()) {
    const source = readText(path);
    if (source !== null) failures.push(...inspectRuntimeText(path, source));
  }
  failures.push(...validateNativeArtifacts());
  failures.push(...validateArchitectureContracts());
  return [...new Set(failures)].sort();
}

if (
  import.meta.url.startsWith("file:") &&
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const failures = runRealComponentCheck();
  if (failures.length > 0) {
    console.error("Real-component verification failed:");
    failures.forEach((failure) => console.error(`- ${failure}`));
    process.exit(1);
  }
  console.log("Real-component verification passed: no production simulation mechanism or marker resource was found.");
}
