#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";
import { validPngThumbnail } from "./p1-contract-gate/artifact-fixture-helpers.mjs";
import { validateMicrosoftImplementationFixture } from "./p1-contract-gate/microsoft-fixture.mjs";
export { validateMicrosoftImplementationFixture } from "./p1-contract-gate/microsoft-fixture.mjs";

const defaultRoot = path.resolve(import.meta.dirname, "..");

export const P1_CONTRACT_NAMES = [
  "ArtifactWorkbook",
  "ArtifactPresentation",
  "DesktopObservation",
  "DesktopAction",
  "MediaAsset",
  "RemoteDevice",
  "CapabilityBundle",
  "LearningCandidate",
  "WorkGraph",
];

const P1_CONTRACT_FIXTURE_KEYS = {
  artifactWorkbook: "ArtifactWorkbook",
  artifactPresentation: "ArtifactPresentation",
  desktopObservation: "DesktopObservation",
  desktopAction: "DesktopAction",
  mediaAsset: "MediaAsset",
  remoteDevice: "RemoteDevice",
  capabilityBundle: "CapabilityBundle",
  learningCandidate: "LearningCandidate",
  workGraph: "WorkGraph",
};

const REQUIRED_P1_DOMAINS = [
  "connectors",
  "workbooks",
  "presentations",
  "computer_use",
  "media",
  "remote_dispatch",
  "bundles",
  "learning",
  "work_graphs",
];

const REQUIRED_THIN_SEAMS = [
  "command_routing",
  "permission_generation",
  "database_migration",
  "navigation",
  "event_projection",
];

const REQUIRED_GATE_KINDS = [
  "production-path",
  "schema-parity",
  "command-parity",
  "import-cycle",
  "module-size",
  "migration",
  "domain-ownership",
  "repository-hygiene",
  "golden-task-contract",
  "external-acceptance",
  "product-contract",
];

const REQUIRED_P0_DOMAINS = [
  "projects",
  "tasks",
  "connectors",
  "routines",
  "browser_automation",
  "artifacts",
  "delegation",
];

const REQUIRED_GATE_COMMANDS = {
  "qualified-matrix": ["npm", "run", "verify:qualified"],
  "real-components": ["npm", "run", "check:real-components"],
  "p0-schema-parity": ["npx", "vitest", "run", "src/lib/__tests__/p0Contracts.test.ts"],
  "p1-typescript-schema-parity": ["npx", "vitest", "run", "src/lib/__tests__/p1Contracts.test.ts"],
  "p1-rust-schema-parity": [
    "cargo",
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--lib",
    "p1_contracts::tests",
  ],
  "command-parity": [
    "cargo",
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--test",
    "capability_parity",
  ],
  "import-cycles": ["npm", "run", "check:module-cycles"],
  "module-size": ["npm", "run", "check:source-size"],
  "migration-integrity": [
    "cargo",
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--lib",
    "migration",
  ],
  "p0-architecture": ["npm", "run", "check:p0-architecture"],
  "repository-hygiene": ["npm", "run", "check:repository-hygiene"],
  "p0-golden-discovery": ["npm", "run", "eval:p0-golden:discovery"],
  "p0-external-acceptance": [
    "node",
    "scripts/p0-release-acceptance.mjs",
    "--evidence-dir=<external>",
    "--build-id=<build>",
    "--source-revision=<revision>",
    "--artifact-digest=<sha256>",
  ],
  "novice-first-ui": ["npm", "run", "check:novice-ui"],
  "p1-microsoft365-ui": [
    "npx", "vitest", "run",
    "src/app/components/integrations/IntegrationsScreen.test.tsx",
    "src/app/components/integrations/microsoft365/Microsoft365ControlPanel.test.tsx",
    "src/app/components/__tests__/SetupJourney.test.tsx",
  ],
  "p1-documents-ui": [
    "npx", "vitest", "run",
    "src/app/components/artifacts/ArtifactStudio.test.tsx",
    "src/app/components/artifacts/review/CreateDocumentAction.test.tsx",
    "src/app/components/chat/ShieldApprovalDialog.test.tsx",
    "src/lib/artifacts/workbooks/schema.test.ts",
  ],
  "p1-presentations-ui": [
    "npx", "vitest", "run",
    "src/app/components/artifacts/ArtifactStudio.test.tsx",
    "src/app/components/artifacts/presentations/PresentationDocumentReview.test.tsx",
    "src/lib/artifacts/presentations/schema.test.ts",
  ],
  "p1-app-control-ui": [
    "npx", "vitest", "run",
    "src/app/components/computer_use/AppControlMonitor.test.tsx",
  ],
  "p1-release-readiness": ["node", "scripts/p1-release-readiness.mjs", "--evidence-dir=<external>", "--build-id=<build>", "--source-revision=<revision>", "--artifact-digest=<sha256>"],
};

const REQUIRED_QUALITY_CHECK_IDS = [
  "real-components",
  "source-line-ratchet",
  "p0-architecture",
  "module-cycles",
  "repository-hygiene",
  "p0-golden-discovery",
  "p0-schema-parity",
  "p0-release-validator-tests",
  "command-parity",
  "migration-integrity",
  "local-bundle-codesign-integrity",
];

const REQUIRED_HERO_IDS = [
  "p1-hero-executive-review",
  "p1-hero-meeting-to-action",
  "p1-hero-teach-once-reuse",
];

const HERO_REQUIREMENTS = {
  "p1-hero-executive-review": {
    implementationSprints: [234, 235, 236, 239, 242, 243],
    requiredContracts: ["RemoteDevice", "ArtifactWorkbook", "ArtifactPresentation", "WorkGraph"],
    postconditionIds: [
      "remote-intent-bound",
      "source-lineage-complete",
      "analysis-budgets-enforced",
      "consequential-actions-approved",
      "workbook-professionally-verified",
      "presentation-professionally-verified",
      "protected-remote-delivery-confirmed",
    ],
    contractDigest: "fa7d592165856829018692fc4b96c66877ad8a1bce28e9da6dc4dac20e337e68",
  },
  "p1-hero-meeting-to-action": {
    implementationSprints: [237, 238, 243],
    requiredContracts: ["MediaAsset", "DesktopObservation", "DesktopAction"],
    postconditionIds: [
      "media-provenance-preserved",
      "transcript-edits-auditable",
      "desktop-observation-fresh",
      "desktop-mutation-authorized",
      "takeover-and-interruption-terminal",
      "application-state-verified",
    ],
    contractDigest: "fb3b53e6989b30beb91252cde75673bb4604abc67dda7c538fda20f5fe7885e2",
  },
  "p1-hero-teach-once-reuse": {
    implementationSprints: [240, 241, 242, 243],
    requiredContracts: ["LearningCandidate", "CapabilityBundle", "WorkGraph"],
    postconditionIds: [
      "candidate-derived-from-success",
      "candidate-reviewed-explicitly",
      "bundle-inspected-before-install",
      "second-project-install-scoped",
      "scope-difference-reviewed",
      "rerun-lineage-and-result-verified",
      "rollback-restores-prior-version",
    ],
    contractDigest: "9806bb85414886a2e6907d72f04ea589f07f5266fef2ab5d76adb303c3da504e",
  },
};

const REQUIRED_P0_IDS = ["projectId", "taskId", "taskRunId", "evidenceIds"];
const REQUIRED_RESOURCE_LIMITS = [
  "tokens",
  "wallTimeMs",
  "memoryBytes",
  "processes",
  "networkRequests",
  "toolCalls",
  "concurrentChildren",
  "mutations",
];
const REQUIRED_RESOURCE_USAGE = [
  "tokens",
  "wallTimeMs",
  "peakMemoryBytes",
  "processes",
  "networkRequests",
  "toolCalls",
  "peakConcurrentChildren",
  "mutationAttempts",
  "mutationsCommitted",
];
const VALID_EVIDENCE_TYPES = new Set([
  "model_assertion",
  "observed_result",
  "executed_mutation",
  "verified_postcondition",
  "signed_artifact",
]);

function loadJson(root, relativePath) {
  return JSON.parse(readFileSync(path.join(root, relativePath), "utf8"));
}

function sameMembers(actual, expected) {
  const left = [...new Set(actual)].sort();
  const right = [...new Set(expected)].sort();
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function heroContractDigest(workflow) {
  return sha256(JSON.stringify({
    implementationSprints: workflow.implementationSprints,
    requiredContracts: workflow.requiredContracts,
    postconditions: (workflow.postconditions ?? []).map(
      ({ id, evidenceClass, requiredFields }) => ({ id, evidenceClass, requiredFields }),
    ),
  }));
}

function validatePathAndSymbol(root, relativePath, symbol, label, failures) {
  const absolute = path.join(root, relativePath);
  if (!existsSync(absolute)) {
    failures.push(`${label}: missing ${relativePath}`);
    return;
  }
  if (!readFileSync(absolute, "utf8").includes(symbol)) {
    failures.push(`${label}: ${relativePath} is missing symbol ${symbol}`);
  }
}

function sourceFilesUnder(directory) {
  if (!existsSync(directory)) return [];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...sourceFilesUnder(absolute));
    else if (entry.isFile() && /\.(?:rs|ts|tsx|mts)$/.test(entry.name)) files.push(absolute);
  }
  return files;
}

function hasExactKeys(value, required, optional = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return required.every((key) => keys.includes(key)) && keys.every((key) => required.includes(key) || optional.includes(key));
}

function validateWorkbookSheet(sheet, label, failures) {
  if (!hasExactKeys(sheet, ["sheetId", "name", "rowCount", "columnCount", "formulaCells", "lineage", "warnings", "previewPageCount"])) {
    failures.push(`${label}: sheet does not match the strict renderer review contract`);
    return;
  }
  for (const cell of sheet.formulaCells ?? []) {
    if (!hasExactKeys(cell, ["address", "status", "sourceRefs"], ["formula", "displayValue"]) || !["calculated", "stale", "error", "unavailable"].includes(cell.status)) {
      failures.push(`${label}: formula cell does not match the normalized review contract`);
    }
  }
  for (const lineage of sheet.lineage ?? []) {
    if (!hasExactKeys(lineage, ["range", "sourceId", "sourceLabel", "freshness"], ["observedAt"])) failures.push(`${label}: lineage entry has an unknown or missing field`);
  }
  for (const warning of sheet.warnings ?? []) {
    if (!hasExactKeys(warning, ["warningId", "severity", "code", "ranges"], ["sheetId"])) failures.push(`${label}: warning has an unknown or missing field`);
  }
}

export function validateWorkbookImplementationFixture(fixture) {
  const failures = [];
  if (!hasExactKeys(fixture, ["schemaVersion", "sprint", "qualificationStatus", "reviews"]) || fixture.schemaVersion !== 1 || fixture.sprint !== 235 || fixture.qualificationStatus !== "contract_tested_not_excel_qualified" || !Array.isArray(fixture.reviews) || fixture.reviews.length === 0) {
    return ["Workbook fixture must be the strict Sprint 235 renderer review envelope"];
  }
  for (const review of fixture.reviews) {
    if (!hasExactKeys(review, ["schemaVersion", "artifactId", "projectId", "taskRunId", "title", "currentRevision", "safePriorRevision", "calculation", "verification", "sheets", "revisions", "versionReviews", "exportable"])) {
      failures.push("Workbook fixture review has engine-only, unknown, or missing renderer fields");
      continue;
    }
    if (review.schemaVersion !== 1 || !hasExactKeys(review.calculation, ["status"]) || !hasExactKeys(review.verification, ["status", "structural", "formula", "visual"])) {
      failures.push(`${review.artifactId}: workbook readiness objects must contain only renderer fields`);
    }
    for (const sheet of review.sheets ?? []) validateWorkbookSheet(sheet, review.artifactId, failures);
    if (!sameMembers((review.revisions ?? []).map((item) => item.revision), (review.versionReviews ?? []).map((item) => item.revision))) {
      failures.push(`${review.artifactId}: revision history and recoverable version reviews differ`);
      continue;
    }
    for (const revision of review.revisions ?? []) {
      if (!hasExactKeys(revision, ["revision", "createdAt", "status", "recoverable"], ["instruction"])) failures.push(`${review.artifactId}: revision history has an unknown or missing field`);
      const version = review.versionReviews.find((item) => item.revision === revision.revision);
      if (!version || !hasExactKeys(version, ["revision", "status", "calculation", "verification", "sheets", "exportable"])) {
        failures.push(`${review.artifactId}: version review has an unknown or missing field`);
        continue;
      }
      if (!hasExactKeys(version.calculation, ["status"]) || !hasExactKeys(version.verification, ["status", "structural", "formula", "visual"])) failures.push(`${review.artifactId}: version readiness has an unknown or missing field`);
      for (const sheet of version.sheets ?? []) validateWorkbookSheet(sheet, `${review.artifactId} version ${version.revision}`, failures);
      const recoverable = version.status !== "building" && version.sheets.some((sheet) => sheet.previewPageCount > 0);
      if (revision.status !== version.status || revision.recoverable !== recoverable) failures.push(`${review.artifactId}: revision status or recoverability contradicts its version review`);
    }
    const current = review.versionReviews.find((item) => item.revision === review.currentRevision);
    if (!current || JSON.stringify(review.calculation) !== JSON.stringify(current.calculation) || JSON.stringify(review.verification) !== JSON.stringify(current.verification) || JSON.stringify(review.sheets) !== JSON.stringify(current.sheets) || review.exportable !== current.exportable) {
      failures.push(`${review.artifactId}: top-level review is not the exact current normalized version`);
    }
    const recoverablePrior = review.revisions.filter((item) => item.revision < review.currentRevision && item.recoverable).map((item) => item.revision);
    const expectedSafePrior = recoverablePrior.length ? Math.max(...recoverablePrior) : undefined;
    if (review.safePriorRevision !== expectedSafePrior) {
      failures.push(`${review.artifactId}: safePriorRevision does not identify the latest recoverable prior version`);
    }
  }
  return failures;
}

export function validatePresentationImplementationFixture(fixture) {
  const failures = [];
  if (!hasExactKeys(fixture, ["schemaVersion", "sprint", "qualificationStatus", "reviews"]) || fixture.schemaVersion !== 1 || fixture.sprint !== 236 || fixture.qualificationStatus !== "contract_tested_not_external_engine_qualified" || !Array.isArray(fixture.reviews) || fixture.reviews.length === 0) {
    return ["Presentation fixture must be the strict Sprint 236 review envelope"];
  }
  const summaryKeys = ["presentationId", "projectId", "taskId", "taskRunId", "artifactId", "title", "currentRevision", "status", "slideCount", "issueCount", "blockerCount", "structurallyVerified", "visuallyVerified", "exportable", "updatedAtMs"];
  const reviewKeys = ["summary", "selectedRevision", "presentation", "revisionHistory", "filmstrip", "issues", "notes", "citations", "provenance", "templateIdentity", "verification"];
  const issueKeys = ["issueId", "revision", "slideId", "code", "severity", "message", "objectId", "evidenceRef"];
  for (const review of fixture.reviews) {
    if (!hasExactKeys(review, reviewKeys) || !hasExactKeys(review.summary, summaryKeys)) {
      failures.push("Presentation review has engine-only, unknown, or missing renderer fields");
      continue;
    }
    const { summary, presentation, verification } = review;
    if (!hasExactKeys(presentation, ["schemaVersion", "title", "locale", "revision", "aspectRatio", "theme", "masters", "layouts", "slides", "citations", "policy", "template"]) || presentation.schemaVersion !== 1 || !Array.isArray(presentation.slides) || presentation.slides.length === 0) {
      failures.push(`${summary.presentationId}: presentation IR is not the strict typed envelope`);
      continue;
    }
    if (review.selectedRevision !== presentation.revision || summary.currentRevision < review.selectedRevision || summary.slideCount !== presentation.slides.length) {
      failures.push(`${summary.presentationId}: selected revision or slide count contradicts its IR`);
    }
    const slideIds = presentation.slides.map((slide) => slide.slideId);
    if (new Set(slideIds).size !== slideIds.length || !sameMembers(slideIds, review.filmstrip.map((slide) => slide.slideId)) || review.filmstrip.some((slide, index) => !hasExactKeys(slide, ["slideId", "position", "title", "layoutId", "issueCount", "blockerCount"], ["thumbnail"]) || slide.position !== index)) {
      failures.push(`${summary.presentationId}: filmstrip must map every selected-revision slide exactly once`);
    }
    for (const slide of review.filmstrip) {
      if (slide.thumbnail && (!hasExactKeys(slide.thumbnail, ["mediaType", "bytesBase64", "width", "height"]) || !validPngThumbnail(slide.thumbnail))) failures.push(`${summary.presentationId}: slide thumbnail is invalid`);
    }
    if (!hasExactKeys(verification, ["packageSha256", "structurallyVerified", "visuallyVerified", "exportable", "checkedAtMs", "renderer", "checks", "issues"]) || !/^[0-9a-f]{64}$/.test(verification.packageSha256 ?? "") || !Array.isArray(verification.checks)) {
      failures.push(`${summary.presentationId}: verification record is invalid`);
      continue;
    }
    for (const check of verification.checks) if (!hasExactKeys(check, ["code", "passed", "detail", "slideId", "objectId"])) failures.push(`${summary.presentationId}: verification check has an unknown or missing field`);
    if (verification.visuallyVerified && !verification.checks.some((check) => check.code === "exact_package_pages_rendered" && check.passed)) failures.push(`${summary.presentationId}: visual readiness lacks exact-package page evidence`);
    for (const issue of review.issues) if (!hasExactKeys(issue, issueKeys) || !["info", "warning", "blocker"].includes(issue.severity) || issue.revision !== review.selectedRevision) failures.push(`${summary.presentationId}: issue is not bound to the selected revision`);
    if (JSON.stringify(review.issues) !== JSON.stringify(verification.issues)) failures.push(`${summary.presentationId}: top-level and verification issues differ`);
    const blockerCount = review.issues.filter((issue) => issue.severity === "blocker").length;
    if (summary.issueCount !== review.issues.length || summary.blockerCount !== blockerCount || summary.structurallyVerified !== verification.structurallyVerified || summary.visuallyVerified !== verification.visuallyVerified || summary.exportable !== verification.exportable) failures.push(`${summary.presentationId}: summary is not the exact selected verification projection`);
    if (verification.exportable !== (verification.structurallyVerified && verification.visuallyVerified && blockerCount === 0) || (verification.visuallyVerified && (!verification.renderer || review.filmstrip.some((slide) => !slide.thumbnail)))) failures.push(`${summary.presentationId}: export or visual readiness is not backed by complete render evidence`);
    if (!sameMembers(review.revisionHistory.map((revision) => revision.revision), [...new Set(review.revisionHistory.map((revision) => revision.revision))]) || !review.revisionHistory.some((revision) => revision.revision === review.selectedRevision)) failures.push(`${summary.presentationId}: revision history is missing or duplicated`);
    const citationKeys = new Set(review.citations.map((citation) => `${citation.slideId}:${citation.objectId ?? ""}:${citation.sourceRef}:${citation.evidenceRef}`));
    for (const provenance of review.provenance) if (!citationKeys.has(`${provenance.slideId}:${provenance.objectId}:${provenance.sourceRef}:${provenance.evidenceRef}`) && !citationKeys.has(`${provenance.slideId}::${provenance.sourceRef}:${provenance.evidenceRef}`)) failures.push(`${summary.presentationId}: provenance lacks an inspectable citation`);
    if (!hasExactKeys(review.templateIdentity, ["templateId", "name", "imported", "fingerprintSha256", "masterIds", "layoutIds"]) || (review.templateIdentity.imported && !/^[0-9a-f]{64}$/.test(review.templateIdentity.fingerprintSha256))) failures.push(`${summary.presentationId}: template identity is incomplete`);
  }
  return failures;
}

export function validateAppControlImplementationFixture(fixture) {
  const failures = [];
  if (!hasExactKeys(fixture, ["schemaVersion", "sprint", "qualificationStatus", "sessions", "evidence", "adversarialChecks"]) || fixture.schemaVersion !== 1 || fixture.sprint !== 237 || fixture.qualificationStatus !== "contract_tested_not_application_matrix_qualified" || !Array.isArray(fixture.sessions) || fixture.sessions.length < 2) {
    return ["App-control fixture must be the strict Sprint 237 guarded-session envelope"];
  }
  const sessionKeys = ["sessionId", "taskRunId", "projectId", "state", "application", "currentAction", "pauseReason", "canPause", "canTakeControl", "canReturnToOomu", "observationGeneration", "lastOutcome", "updatedAtMs"];
  const states = ["observing", "running", "paused", "takeover", "return_pending", "stopped", "completed", "failed"];
  const actions = ["focus", "press", "select", "type_text", "invoke_menu", "scroll", "drag_drop", "choose_file", "apple_event"];
  for (const session of fixture.sessions) {
    if (!hasExactKeys(session, sessionKeys) || !states.includes(session.state) || !Number.isInteger(session.observationGeneration) || session.observationGeneration < 1) {
      failures.push("App-control session has an unknown, missing, or invalid field");
      continue;
    }
    if (session.application && !hasExactKeys(session.application, ["name", "icon"])) failures.push(`${session.sessionId}: application view is not display-only`);
    if (session.currentAction && (!hasExactKeys(session.currentAction, ["kind", "targetLabel", "willChangeData"]) || !actions.includes(session.currentAction.kind))) failures.push(`${session.sessionId}: current action view is invalid`);
    if (session.lastOutcome && (!hasExactKeys(session.lastOutcome, ["status", "actionKind", "receiptId", "recordedAtMs", "detailsAvailable"]) || !actions.includes(session.lastOutcome.actionKind))) failures.push(`${session.sessionId}: last outcome view is invalid`);
    if (session.state === "paused" && !session.pauseReason) failures.push(`${session.sessionId}: paused session requires a mapped reason`);
    if (session.state === "takeover" && !session.canReturnToOomu) failures.push(`${session.sessionId}: takeover must offer an explicit return handoff`);
  }
  const evidenceKeys = ["freshObservationRequired", "postActionObservationRequired", "taskBoundReferences", "applicationProcessWindowBoundReferences", "referenceTtlMs", "takeoverInvalidatesReferences", "returnRequiresReobservation", "staleReplayAllowed"];
  if (!hasExactKeys(fixture.evidence, evidenceKeys) || evidenceKeys.filter((key) => key !== "referenceTtlMs" && key !== "staleReplayAllowed").some((key) => fixture.evidence[key] !== true) || fixture.evidence.staleReplayAllowed !== false || !Number.isInteger(fixture.evidence.referenceTtlMs) || fixture.evidence.referenceTtlMs < 1 || fixture.evidence.referenceTtlMs > 30_000) failures.push("App-control evidence must require fresh, bounded, Task-bound observations with no stale replay");
  const adversarialKeys = ["crossApplicationReferenceRejected", "secureFieldRejected", "staleReferenceRejected", "hiddenWindowRejected", "rawScriptRejected", "rawCoordinatesRejected", "unapprovedFileRejected", "browserRouteRejected", "missingPermissionRejected", "userInputPausesImmediately"];
  if (!hasExactKeys(fixture.adversarialChecks, adversarialKeys) || adversarialKeys.some((key) => fixture.adversarialChecks[key] !== true)) failures.push("Every Sprint 237 adversarial check must pass fail-closed");
  if (!fixture.sessions.some((session) => session.state === "paused" && session.pauseReason === "secure_field") || !fixture.sessions.some((session) => session.state === "completed" && session.lastOutcome?.status === "verified")) failures.push("Fixture must cover a protected pause and a verified completion");
  return failures;
}

export function validateLaterImplementationFixture(fixture, sprint) {
  if (!hasExactKeys(fixture, ["schemaVersion", "sprint", "qualificationStatus", "contracts", "checks"]) || fixture.schemaVersion !== 1 || fixture.sprint !== sprint || !String(fixture.qualificationStatus ?? "").includes("not_") || !Array.isArray(fixture.contracts) || fixture.contracts.length === 0 || !fixture.checks || typeof fixture.checks !== "object" || Array.isArray(fixture.checks)) {
    return [`Sprint ${sprint} fixture must be a strict contract-only evidence envelope`];
  }
  const checks = Object.values(fixture.checks);
  if (checks.length < 4 || checks.some((value) => value !== true)) return [`Sprint ${sprint} fixture must bind at least four passing in-repository checks`];
  return [];
}

export function validateOwnership(root, ownership) {
  const failures = [];
  if (
    ownership.schemaVersion !== 2 ||
    ownership.sprint !== 243 ||
    ownership.contractFreezeSprint !== 233 ||
    ownership.latestImplementedSprint !== 243
  ) {
    failures.push("P1 ownership must use the Sprint 243 implementation-state manifest");
  }
  if (ownership.maximumLinesPerDomainFile !== 750) {
    failures.push("P1 ownership must retain the reviewed 750-line domain ceiling");
  }
  const domains = ownership.domains ?? [];
  if (!sameMembers(domains.map((domain) => domain.name), REQUIRED_P1_DOMAINS)) {
    failures.push("P1 ownership must reserve exactly the nine named domains");
  }
  const roots = new Set();
  const cycleNodes = new Set();
  const coveredContracts = [];
  const ownershipDocPath = path.join(root, "docs/architecture/p1-contract-ownership.md");
  const ownershipDoc = existsSync(ownershipDocPath) ? readFileSync(ownershipDocPath, "utf8") : "";
  if (!ownershipDoc) failures.push("P1 ownership documentation is missing");
  for (const domain of domains) {
    if (!Number.isInteger(domain.implementationSprint) || domain.implementationSprint < 234 || domain.implementationSprint > 242) {
      failures.push(`${domain.name}: implementation sprint must be between 234 and 242`);
    }
    if (typeof domain.owner !== "string" || domain.owner.trim().length === 0) {
      failures.push(`${domain.name}: a named owner is required`);
    }
    const implementedNow = domain.implementationSprint <= ownership.sprint;
    if (implementedNow) {
      if (
        domain.reservationOnly !== false ||
        domain.implemented !== true ||
        domain.implementationStatus !== "implemented" ||
        domain.qualificationStatus !== "contract-verified-external-not-run"
      ) {
        failures.push(`${domain.name}: implemented roots require truthful contract-only qualification state`);
      }
      if (typeof domain.contractFixture !== "string" || !existsSync(path.join(root, domain.contractFixture))) {
        failures.push(`${domain.name}: implemented root contract fixture is missing`);
      } else {
        const fixture = loadJson(root, domain.contractFixture);
        if (fixture.sprint !== domain.implementationSprint || !String(fixture.qualificationStatus ?? "").includes("not_")) {
          failures.push(`${domain.name}: fixture must identify its sprint and unqualified external state`);
        }
      }
    } else if (domain.reservationOnly !== true || domain.implemented !== false) {
      failures.push(`${domain.name}: future implementation roots must remain reservation-only`);
    }
    if (domain.name === "connectors") {
      if (
        domain.contract !== "ConnectorId" ||
        domain.contractSource !== "p0" ||
        !sameMembers(domain.reusedP0Contracts ?? [], ["ConnectorAccount", "ProjectConnectorBinding"])
      ) {
        failures.push("connectors: P1 must explicitly reuse the P0 connector identity, account, and binding contracts");
      }
    } else if (domain.contractSource !== "p1") {
      failures.push(`${domain.name}: P1-owned contract source is required`);
    }
    if (!Array.isArray(domain.roots) || domain.roots.length < 2) {
      failures.push(`${domain.name}: native and renderer/lib future roots are required`);
    }
    for (const rootPath of domain.roots ?? []) {
      if (path.isAbsolute(rootPath) || rootPath.includes("..") || roots.has(rootPath)) {
        failures.push(`${domain.name}: invalid or duplicate future root ${rootPath}`);
      }
      roots.add(rootPath);
      if (ownershipDoc && !ownershipDoc.includes(`\`${rootPath}/\``)) {
        failures.push(`${domain.name}: documented roots do not include ${rootPath}`);
      }
      for (const sourcePath of sourceFilesUnder(path.join(root, rootPath))) {
        const lineCount = readFileSync(sourcePath, "utf8").split("\n").length - 1;
        if (lineCount > ownership.maximumLinesPerDomainFile) {
          failures.push(
            `${path.relative(root, sourcePath)}: ${lineCount} lines exceeds P1 domain ceiling ${ownership.maximumLinesPerDomainFile}`,
          );
        }
      }
    }
    const expectedCycleNodes = (domain.roots ?? []).map((rootPath) =>
      rootPath.startsWith("src-tauri/src/")
        ? `rust:${rootPath.slice("src-tauri/src/".length).replaceAll("/", "::")}`
        : `ts:${rootPath}`,
    );
    if (
      !Array.isArray(domain.cycleNodes) ||
      domain.cycleNodes.length !== (domain.roots ?? []).length ||
      !sameMembers(domain.cycleNodes, expectedCycleNodes)
    ) {
      failures.push(`${domain.name}: every reserved root requires one matching cycle node`);
    }
    for (const node of domain.cycleNodes ?? []) {
      if (!/^(?:rust|ts):/.test(node) || cycleNodes.has(node)) {
        failures.push(`${domain.name}: invalid or duplicate cycle node ${node}`);
      }
      cycleNodes.add(node);
    }
    if (domain.contractSource === "p1") {
      coveredContracts.push(domain.contract, ...(domain.secondaryContracts ?? []));
    }
  }
  if (
    !sameMembers(coveredContracts, P1_CONTRACT_NAMES) ||
    new Set(coveredContracts).size !== coveredContracts.length
  ) {
    failures.push("P1 ownership does not cover the exact frozen contract inventory");
  }

  const sharedContracts = ownership.sharedContractFiles ?? [];
  if (!sameMembers(sharedContracts.map((item) => item.path), ["src/lib/p1Contracts.ts", "src-tauri/src/p1_contracts.rs"])) {
    failures.push("P1 ownership must reserve both central shared contract files");
  }
  for (const item of sharedContracts) {
    if (item.maximumLines !== 1500 || typeof item.owner !== "string" || !item.owner.trim()) {
      failures.push(`${item.path}: shared contract owner and 1500-line ceiling are required`);
      continue;
    }
    const absolute = path.join(root, item.path);
    if (!existsSync(absolute)) {
      failures.push(`${item.path}: shared contract file is missing`);
      continue;
    }
    const lineCount = readFileSync(absolute, "utf8").split("\n").length - 1;
    if (lineCount > item.maximumLines) {
      failures.push(`${item.path}: ${lineCount} lines exceeds shared contract ceiling ${item.maximumLines}`);
    }
  }

  const seams = ownership.thinSharedSeams ?? [];
  if (!sameMembers(seams.map((seam) => seam.purpose), REQUIRED_THIN_SEAMS)) {
    failures.push("P1 ownership must reserve the five thin shared seams");
  }
  for (const seam of seams) {
    if (seam.editPolicy !== "thin-registration-only") {
      failures.push(`${seam.purpose}: shared seam is not registration-only`);
    }
    validatePathAndSymbol(
      root,
      seam.path,
      seam.requiredSymbol,
      `thin seam ${seam.purpose}`,
      failures,
    );
  }
  return failures;
}

function validatePublicQualityPolicy(root, manifest) {
  const policyPath = manifest.qualityPolicyPath === "release/quality-gate-policy.json"
    ? path.join(root, manifest.qualityPolicyPath)
    : null;
  if (!policyPath || !existsSync(policyPath)) {
    return ["P1 gate manifest must bind the public quality-gate policy"];
  }
  const policy = JSON.parse(readFileSync(policyPath, "utf8"));
  const complete = policy.schemaVersion === 1
    && policy.kind === "oomu.public-quality-gate-policy"
    && policy.releaseClaim === false
    && sameMembers(policy.requiredChecks ?? [], REQUIRED_QUALITY_CHECK_IDS)
    && new Set(policy.requiredChecks ?? []).size === REQUIRED_QUALITY_CHECK_IDS.length
    && sameMembers(
      policy.requiredExternalQualification ?? [],
      ["signed-notarized-release", "fresh-install-hero", "external-channel-acceptance"],
    )
    && policy.protectedSurfaceDigest === manifest.protectedSurfaceDigest;
  return complete
    ? []
    : ["Public quality-gate policy is incomplete or does not match the protected surface"];
}

export function validateGateManifest(root, manifest, { allowMissingSharedFixture = false, protectedContentOverrides = {} } = {}) {
  const failures = [];
  const deferred = [];
  if (manifest.schemaVersion !== 2 || manifest.sprint !== 243
    || manifest.contractFreezeSprint !== 233 || manifest.latestImplementedSprint !== 243) {
    failures.push("P1 gate manifest must use the Sprint 243 implementation-state manifest");
  }
  failures.push(...validatePublicQualityPolicy(root, manifest));
  if (!/^sha256:[0-9a-f]{64}$/.test(manifest.protectedSurfaceDigest ?? "")) {
    failures.push("P1 gate manifest must pin the current protected contract surface");
  } else if (
    computeProtectedSurfaceDigest(root, protectedContentOverrides)
      !== manifest.protectedSurfaceDigest
  ) {
    failures.push("Protected P0/shared contract surface has drifted");
  }
  if (!sameMembers(manifest.contractNames ?? [], P1_CONTRACT_NAMES)) {
    failures.push("P1 gate manifest contract inventory differs from the frozen inventory");
  }
  if (!sameMembers(manifest.protectedP0Domains ?? [], REQUIRED_P0_DOMAINS)) {
    failures.push("P1 gate manifest must protect the exact seven P0 domains");
  }
  const gates = manifest.gates ?? [];
  if (!sameMembers(gates.map((gate) => gate.kind), REQUIRED_GATE_KINDS)) {
    failures.push("P1 gate manifest does not bind every required ratchet kind");
  }
  const gateIds = new Set();
  if (!sameMembers(gates.map((gate) => gate.id), Object.keys(REQUIRED_GATE_COMMANDS))) {
    failures.push("P1 gate manifest has a missing or unknown gate id");
  }
  for (const gate of gates) {
    if (gateIds.has(gate.id)) failures.push(`duplicate gate id: ${gate.id}`);
    gateIds.add(gate.id);
    if (!Array.isArray(gate.command) || gate.command.length < 2) {
      failures.push(`${gate.id}: executable command mapping is required`);
    } else if (JSON.stringify(gate.command) !== JSON.stringify(REQUIRED_GATE_COMMANDS[gate.id])) {
      failures.push(`${gate.id}: command differs from the frozen production gate`);
    }
    if (typeof gate.postcondition !== "string" || gate.postcondition.length < 24) {
      failures.push(`${gate.id}: concrete observable postcondition is required`);
    }
    validatePathAndSymbol(root, gate.path, gate.requiredSymbol, `gate ${gate.id}`, failures);
  }

  const implementationFixtures = manifest.implementationFixtures ?? [];
  if (!sameMembers(implementationFixtures.map((item) => item.sprint), [234, 235, 236, 237, 238, 239, 240, 241, 242])) {
    failures.push("P1 implementation fixtures must cover exactly Sprints 234 through 242");
  }
  for (const item of implementationFixtures) {
    if (!String(item.qualificationStatus ?? "").includes("not-") || !existsSync(path.join(root, item.path ?? ""))) {
      failures.push(`Sprint ${item.sprint}: fixture must exist and must not claim external qualification`);
      continue;
    }
    const fixture = loadJson(root, item.path);
    if (fixture.sprint !== item.sprint || !String(fixture.qualificationStatus ?? "").includes("not_")) {
      failures.push(`Sprint ${item.sprint}: fixture qualification state is inconsistent`);
    }
    const fixtureFailures = item.sprint === 234
      ? validateMicrosoftImplementationFixture(fixture)
      : item.sprint === 235
        ? validateWorkbookImplementationFixture(fixture)
        : item.sprint === 236
          ? validatePresentationImplementationFixture(fixture)
          : item.sprint === 237
            ? validateAppControlImplementationFixture(fixture)
            : validateLaterImplementationFixture(fixture, item.sprint);
    failures.push(...fixtureFailures.map((failure) => `Sprint ${item.sprint}: ${failure}`));
  }

  const fixturePath = path.join(root, manifest.sharedContractFixturePath);
  if (!existsSync(fixturePath)) {
    const message = `shared P1 contract fixture is missing: ${manifest.sharedContractFixturePath}`;
    if (allowMissingSharedFixture) deferred.push(message);
    else failures.push(message);
  } else {
    const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
    if (fixture.schemaVersion !== 1 || !fixture.contracts || Array.isArray(fixture.contracts)) {
      failures.push("shared P1 fixture must expose version 1 contract vectors");
    }
    const fixtureContracts = Object.keys(fixture.contracts ?? {}).map(
      (key) => P1_CONTRACT_FIXTURE_KEYS[key] ?? `UNKNOWN:${key}`,
    );
    if (!sameMembers(fixtureContracts, P1_CONTRACT_NAMES)) {
      failures.push("shared P1 fixture does not contain exactly the frozen contract inventory");
    }
  }

  const parity = manifest.crossLanguageParity ?? {};
  for (const [label, pathKey, symbolKey] of [
    ["P1 Rust contracts", "rustPath", "rustRequiredSymbol"],
    ["P1 TypeScript contracts", "typescriptPath", "typescriptRequiredSymbol"],
    ["P1 Rust parity tests", "rustTestPath", "rustTestRequiredSymbol"],
    ["P1 TypeScript parity tests", "typescriptTestPath", "typescriptTestRequiredSymbol"],
  ]) {
    const relativePath = parity[pathKey];
    const requiredSymbol = parity[symbolKey];
    if (typeof relativePath !== "string" || typeof requiredSymbol !== "string") {
      failures.push(`${label}: path and required symbol are required`);
      continue;
    }
    if (!existsSync(path.join(root, relativePath)) && allowMissingSharedFixture) {
      deferred.push(`${label} is missing: ${relativePath}`);
      continue;
    }
    validatePathAndSymbol(root, relativePath, requiredSymbol, label, failures);
    if (existsSync(path.join(root, relativePath))) {
      const source = readFileSync(path.join(root, relativePath), "utf8");
      for (const contractName of P1_CONTRACT_NAMES) {
        if (!source.includes(contractName)) {
          failures.push(`${label}: ${relativePath} is missing ${contractName}`);
        }
      }
    }
  }
  validatePathAndSymbol(
    root,
    "src-tauri/src/lib.rs",
    "pub mod p1_contracts;",
    "P1 Rust production registration",
    failures,
  );
  for (const [label, symbol] of [
    ["qualified novice-first UI registration", '["novice-first-ui", "npm", ["run", "check:novice-ui"]]'],
    ["qualified Microsoft UI registration", '["p1-microsoft365-ui", "npx", ["vitest", "run", "src/app/components/integrations/IntegrationsScreen.test.tsx", "src/app/components/integrations/microsoft365/Microsoft365ControlPanel.test.tsx", "src/app/components/__tests__/SetupJourney.test.tsx"]]'],
    ["qualified Documents UI registration", '["p1-documents-ui", "npx", ["vitest", "run", "src/app/components/artifacts/ArtifactStudio.test.tsx", "src/app/components/artifacts/review/CreateDocumentAction.test.tsx", "src/app/components/chat/ShieldApprovalDialog.test.tsx", "src/lib/artifacts/workbooks/schema.test.ts"]]'],
  ]) {
    validatePathAndSymbol(root, "scripts/verification-matrix.mjs", symbol, label, failures);
  }
  validatePathAndSymbol(
    root,
    "scripts/verification-matrix.mjs",
    '["p1-typescript-schema-parity", "npx", ["vitest", "run", "src/lib/__tests__/p1Contracts.test.ts"]]',
    "qualified P1 TypeScript parity registration",
    failures,
  );
  validatePathAndSymbol(
    root,
    "scripts/verification-matrix.mjs",
    '["p1-rust-schema-parity", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "p1_contracts::tests"]]',
    "qualified P1 Rust parity registration",
    failures,
  );
  validatePathAndSymbol(
    root,
    "scripts/verification-matrix.mjs",
    '["p1-contracts", "npm", ["run", "check:p1-contracts"]]',
    "qualified P1 gate registration",
    failures,
  );
  const packageManifest = loadJson(root, "package.json");
  if (packageManifest.scripts?.["check:p1-contracts"] !== "node scripts/check-p1-contract-gate.mjs") {
    failures.push("package.json: check:p1-contracts must invoke the canonical P1 contract gate");
  }
  return { failures, deferred };
}

export function validateHeroDefinitions(definitions) {
  const failures = [];
  if (definitions.schemaVersion !== 1 || definitions.definitionStatus !== "contract_only" || definitions.executable !== false) {
    failures.push("P1 hero definitions must remain non-executable Sprint 233 contracts");
  }
  if (
    !sameMembers(definitions.requiredP0Ids ?? [], REQUIRED_P0_IDS) ||
    new Set(definitions.requiredP0Ids ?? []).size !== (definitions.requiredP0Ids ?? []).length
  ) {
    failures.push("P1 heroes must reuse the four required P0 identity fields");
  }
  if (
    !sameMembers(definitions.allowedEvidenceClasses ?? [], [...VALID_EVIDENCE_TYPES]) ||
    new Set(definitions.allowedEvidenceClasses ?? []).size !==
      (definitions.allowedEvidenceClasses ?? []).length
  ) {
    failures.push("P1 hero evidence type inventory is incomplete");
  }
  const workflows = definitions.workflows ?? [];
  if (
    !sameMembers(workflows.map((workflow) => workflow.id), REQUIRED_HERO_IDS) ||
    new Set(workflows.map((workflow) => workflow.id)).size !== workflows.length
  ) {
    failures.push("exactly the three P1 hero workflows are required");
  }
  for (const workflow of workflows) {
    const expected = HERO_REQUIREMENTS[workflow.id];
    if ("productionCommands" in workflow || workflow.executed === true || workflow.available === true) {
      failures.push(`${workflow.id}: Sprint 233 cannot imply later-sprint behavior`);
    }
    if (
      !expected ||
      !Array.isArray(workflow.implementationSprints) ||
      workflow.implementationSprints.length === 0 ||
      new Set(workflow.implementationSprints).size !== workflow.implementationSprints.length ||
      !sameMembers(workflow.implementationSprints, expected?.implementationSprints ?? []) ||
      workflow.implementationSprints.some((sprint) => sprint < 234 || sprint > 243)
    ) {
      failures.push(`${workflow.id}: later implementation sprint ownership is required`);
    }
    if (
      !expected ||
      !Array.isArray(workflow.requiredContracts) ||
      workflow.requiredContracts.length === 0 ||
      new Set(workflow.requiredContracts).size !== workflow.requiredContracts.length ||
      !sameMembers(workflow.requiredContracts, expected?.requiredContracts ?? [])
    ) {
      failures.push(`${workflow.id}: required P1 contracts are incomplete`);
    }
    for (const contract of workflow.requiredContracts ?? []) {
      if (!P1_CONTRACT_NAMES.includes(contract)) {
        failures.push(`${workflow.id}: unknown contract ${contract}`);
      }
    }
    if (
      !expected ||
      !Array.isArray(workflow.postconditions) ||
      !sameMembers(workflow.postconditions.map((item) => item.id), expected?.postconditionIds ?? []) ||
      workflow.postconditions.length !== (expected?.postconditionIds.length ?? 0)
    ) {
      failures.push(`${workflow.id}: exact observable postconditions are required`);
    }
    const ids = new Set();
    for (const postcondition of workflow.postconditions ?? []) {
      if (ids.has(postcondition.id)) failures.push(`${workflow.id}: duplicate postcondition ${postcondition.id}`);
      ids.add(postcondition.id);
      if (!VALID_EVIDENCE_TYPES.has(postcondition.evidenceClass)) {
        failures.push(`${workflow.id}/${postcondition.id}: invalid evidence type`);
      }
      if (typeof postcondition.observable !== "string" || postcondition.observable.length < 48) {
        failures.push(`${workflow.id}/${postcondition.id}: observable result is not concrete`);
      }
      if (!Array.isArray(postcondition.requiredFields) || postcondition.requiredFields.length < 3) {
        failures.push(`${workflow.id}/${postcondition.id}: at least three observable evidence fields are required`);
      } else if (
        new Set(postcondition.requiredFields).size !== postcondition.requiredFields.length ||
        postcondition.requiredFields.some(
          (field) => typeof field !== "string" || !/^[A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z][A-Za-z0-9]*)*$/.test(field),
        )
      ) {
        failures.push(`${workflow.id}/${postcondition.id}: evidence fields must be nonempty and unique`);
      }
    }
    if (expected && heroContractDigest(workflow) !== expected.contractDigest) {
      failures.push(`${workflow.id}: frozen contracts, evidence classes, or field paths changed`);
    }
  }
  return failures;
}

export function validateTelemetrySchema(schema) {
  const failures = [];
  const expectedTopLevel = [
    "schemaVersion",
    "sampleId",
    "build",
    "task",
    "phase",
    "sampledAt",
    "intervalMs",
    "process",
    "system",
    "resourceBudget",
    "routingObservation",
    "measurement",
  ];
  if (schema.additionalProperties !== false || !sameMembers(schema.required ?? [], expectedTopLevel)) {
    failures.push("telemetry schema must require the complete closed evidence envelope");
  }
  const budget = schema.properties?.resourceBudget;
  if (!sameMembers(budget?.properties?.limits?.required ?? [], REQUIRED_RESOURCE_LIMITS)) {
    failures.push("telemetry schema resource limits must match the shared P1 budget contract");
  }
  if (!sameMembers(budget?.properties?.usage?.required ?? [], REQUIRED_RESOURCE_USAGE)) {
    failures.push("telemetry schema resource usage must match the shared P1 budget contract");
  }
  if (!sameMembers(budget?.required ?? [], ["limits", "usage", "sampledAt"]) || budget?.properties?.sampledAt?.format !== "date-time") {
    failures.push("telemetry schema resource budget envelope is incomplete");
  }
  if (
    budget?.additionalProperties !== true ||
    budget?.properties?.limits?.additionalProperties !== true ||
    budget?.properties?.usage?.additionalProperties !== true
  ) {
    failures.push("shared resource budget telemetry must preserve forward optional fields");
  }
  if (
    budget?.properties?.limits?.properties?.concurrentChildren?.minimum !== 1 ||
    budget?.properties?.limits?.properties?.concurrentChildren?.maximum !== 8 ||
    budget?.properties?.usage?.properties?.peakConcurrentChildren?.minimum !== 0 ||
    budget?.properties?.usage?.properties?.peakConcurrentChildren?.maximum !== 8
  ) {
    failures.push("telemetry schema must preserve the bounded concurrency constraints");
  }
  for (const field of REQUIRED_RESOURCE_LIMITS) {
    const property = budget?.properties?.limits?.properties?.[field];
    const expectedMinimum = field === "concurrentChildren" ? 1 : 0;
    if (property?.type !== "integer" || property?.minimum !== expectedMinimum) {
      failures.push(`telemetry schema limit ${field} lost its integer/minimum constraint`);
    }
  }
  for (const field of REQUIRED_RESOURCE_USAGE) {
    const property = budget?.properties?.usage?.properties?.[field];
    if (property?.type !== "integer" || property?.minimum !== 0) {
      failures.push(`telemetry schema usage ${field} lost its integer/minimum constraint`);
    }
  }
  if (schema.properties?.routingObservation?.properties?.policyMutation?.const !== false) {
    failures.push("telemetry schema must prohibit routing-policy mutation");
  }
  if (schema.properties?.measurement?.properties?.source?.const !== "observed") {
    failures.push("telemetry schema must identify measurements as observed");
  }
  if (
    !sameMembers(schema.properties?.build?.required ?? [], ["buildIdentifier", "sourceRevision"]) ||
    schema.properties?.build?.properties?.sourceRevision?.pattern !== "^[0-9a-f]{40}$"
  ) {
    failures.push("telemetry schema must bind a build identifier and source revision");
  }
  if (!sameMembers(schema.properties?.task?.required ?? [], ["projectId", "taskId", "taskRunId"])) {
    failures.push("telemetry schema must bind the existing P0 Task identities");
  }
  for (const [field, prefix] of [["projectId", "project"], ["taskId", "task"], ["taskRunId", "taskrun"]]) {
    if (!new RegExp(`^\\^${prefix}_`).test(schema.properties?.task?.properties?.[field]?.pattern ?? "")) {
      failures.push(`telemetry schema ${field} must enforce the P0 prefix`);
    }
  }
  const phases = ["startup", "idle", "capture", "automation", "delegation", "rendering", "analysis", "remote_sync", "shutdown"];
  if (
    !sameMembers(schema.properties?.phase?.enum ?? [], phases) ||
    schema.properties?.sampledAt?.format !== "date-time" ||
    schema.properties?.intervalMs?.minimum !== 1
  ) {
    failures.push("telemetry schema must constrain phase, timestamp, and sampling interval");
  }
  if (!sameMembers(schema.properties?.process?.required ?? [], [
    "pid",
    "cpuTimeMs",
    "cpuPercent",
    "residentMemoryBytes",
    "peakResidentMemoryBytes",
    "threadCount",
    "childProcessCount",
  ])) {
    failures.push("telemetry schema process measurements are incomplete");
  }
  for (const [field, expectedType, minimum] of [
    ["pid", "integer", 1],
    ["cpuTimeMs", "integer", 0],
    ["cpuPercent", "number", 0],
    ["residentMemoryBytes", "integer", 0],
    ["peakResidentMemoryBytes", "integer", 0],
    ["threadCount", "integer", 1],
    ["childProcessCount", "integer", 0],
  ]) {
    const property = schema.properties?.process?.properties?.[field];
    if (property?.type !== expectedType || property?.minimum !== minimum) {
      failures.push(`telemetry schema process.${field} lost its type/minimum constraint`);
    }
  }
  if (!sameMembers(schema.properties?.system?.required ?? [], [
    "cpuPercent",
    "memoryUsedBytes",
    "memoryAvailableBytes",
    "thermalState",
    "powerSource",
  ])) {
    failures.push("telemetry schema system measurements are incomplete");
  }
  for (const [field, expectedType] of [["cpuPercent", "number"], ["memoryUsedBytes", "integer"], ["memoryAvailableBytes", "integer"]]) {
    const property = schema.properties?.system?.properties?.[field];
    if (property?.type !== expectedType || property?.minimum !== 0) {
      failures.push(`telemetry schema system.${field} lost its type/minimum constraint`);
    }
  }
  if (
    !sameMembers(schema.properties?.system?.properties?.thermalState?.enum ?? [], ["nominal", "fair", "serious", "critical", "unavailable"]) ||
    !sameMembers(schema.properties?.system?.properties?.powerSource?.enum ?? [], ["battery", "external", "unavailable"])
  ) {
    failures.push("telemetry schema system state enums are incomplete");
  }
  if (
    !sameMembers(schema.properties?.routingObservation?.required ?? [], ["providerId", "modelId", "routeClass", "policyMutation"]) ||
    !sameMembers(schema.properties?.routingObservation?.properties?.routeClass?.enum ?? [], ["local", "cloud"])
  ) {
    failures.push("telemetry schema routing observation is incomplete");
  }
  if (!sameMembers(schema.properties?.measurement?.required ?? [], ["collector", "source", "clock", "monotonicSequence"])) {
    failures.push("telemetry schema measurement provenance is incomplete");
  }
  if (
    schema.properties?.measurement?.properties?.monotonicSequence?.type !== "integer" ||
    schema.properties?.measurement?.properties?.monotonicSequence?.minimum !== 0 ||
    !sameMembers(schema.properties?.measurement?.properties?.clock?.enum ?? [], ["monotonic", "wall_and_monotonic"])
  ) {
    failures.push("telemetry schema measurement constraints are incomplete");
  }
  return failures;
}

export function validateTelemetryRecord(record) {
  const failures = [];
  if (record.schemaVersion !== 1) failures.push("telemetry record has an unsupported version");
  const utc = (value) => typeof value === "string" && value.endsWith("Z") && !Number.isNaN(Date.parse(value));
  if (
    typeof record.build?.buildIdentifier !== "string" ||
    !record.build.buildIdentifier.trim() ||
    !/^[0-9a-f]{40}$/.test(record.build?.sourceRevision ?? "")
  ) {
    failures.push("telemetry record is not bound to a valid build");
  }
  if (!new Set(["startup", "idle", "capture", "automation", "delegation", "rendering", "analysis", "remote_sync", "shutdown"]).has(record.phase)) {
    failures.push("telemetry record has an invalid or missing phase");
  }
  if (!utc(record.sampledAt) || !Number.isInteger(record.intervalMs) || record.intervalMs < 1) {
    failures.push("telemetry record requires a valid timestamp and positive interval");
  }
  if (record.routingObservation?.policyMutation !== false) {
    failures.push("telemetry record attempted to mutate routing policy");
  }
  if (record.measurement?.source !== "observed") {
    failures.push("telemetry record is not backed by an observed measurement");
  }
  const idPattern = (prefix) => new RegExp(`^${prefix}_[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`);
  for (const [field, prefix] of [["projectId", "project"], ["taskId", "task"], ["taskRunId", "taskrun"]]) {
    if (!idPattern(prefix).test(record.task?.[field] ?? "")) {
      failures.push(`${field}: telemetry must reuse the P0 identifier format`);
    }
  }
  if (record.task?.workGraphId !== undefined && !idPattern("workgraph").test(record.task.workGraphId)) {
    failures.push("workGraphId: telemetry must reuse the P1 identifier format");
  }
  for (const [field, minimum] of [
    ["pid", 1],
    ["cpuTimeMs", 0],
    ["residentMemoryBytes", 0],
    ["peakResidentMemoryBytes", 0],
    ["threadCount", 1],
    ["childProcessCount", 0],
  ]) {
    if (!Number.isInteger(record.process?.[field]) || record.process[field] < minimum) {
      failures.push(`process.${field}: required observed integer is missing or invalid`);
    }
  }
  if (typeof record.process?.cpuPercent !== "number" || record.process.cpuPercent < 0) {
    failures.push("process.cpuPercent: required observed measurement is missing or invalid");
  }
  if (
    Number.isInteger(record.process?.peakResidentMemoryBytes) &&
    Number.isInteger(record.process?.residentMemoryBytes) &&
    record.process.peakResidentMemoryBytes < record.process.residentMemoryBytes
  ) {
    failures.push("process peak resident memory cannot be below current resident memory");
  }
  for (const field of ["memoryUsedBytes", "memoryAvailableBytes"]) {
    if (!Number.isInteger(record.system?.[field]) || record.system[field] < 0) {
      failures.push(`system.${field}: required observed integer is missing or invalid`);
    }
  }
  if (typeof record.system?.cpuPercent !== "number" || record.system.cpuPercent < 0) {
    failures.push("system.cpuPercent: required observed measurement is missing or invalid");
  }
  if (!new Set(["nominal", "fair", "serious", "critical", "unavailable"]).has(record.system?.thermalState)) {
    failures.push("system.thermalState is missing or invalid");
  }
  if (!new Set(["battery", "external", "unavailable"]).has(record.system?.powerSource)) {
    failures.push("system.powerSource is missing or invalid");
  }
  const limits = record.resourceBudget?.limits ?? {};
  const usage = record.resourceBudget?.usage ?? {};
  if (!utc(record.resourceBudget?.sampledAt)) {
    failures.push("resource budget telemetry requires a valid observed timestamp");
  }
  for (const field of REQUIRED_RESOURCE_LIMITS) {
    if (!Number.isInteger(limits[field]) || limits[field] < 0) {
      failures.push(`${field}: a non-negative integer resource limit is required`);
    }
  }
  for (const field of REQUIRED_RESOURCE_USAGE) {
    if (!Number.isInteger(usage[field]) || usage[field] < 0) {
      failures.push(`${field}: non-negative integer observed usage is required`);
    }
  }
  for (const [usageField, limitField] of [
    ["tokens", "tokens"],
    ["wallTimeMs", "wallTimeMs"],
    ["peakMemoryBytes", "memoryBytes"],
    ["processes", "processes"],
    ["networkRequests", "networkRequests"],
    ["toolCalls", "toolCalls"],
    ["peakConcurrentChildren", "concurrentChildren"],
    ["mutationAttempts", "mutations"],
    ["mutationsCommitted", "mutations"],
  ]) {
    if (Number.isInteger(usage[usageField]) && Number.isInteger(limits[limitField]) && usage[usageField] > limits[limitField]) {
      failures.push(`${usageField}: observed usage exceeds the declared ${limitField} limit`);
    }
  }
  if (usage.mutationsCommitted > usage.mutationAttempts) {
    failures.push("mutationsCommitted cannot exceed mutationAttempts");
  }
  if (
    typeof record.routingObservation?.providerId !== "string" ||
    !record.routingObservation.providerId.trim() ||
    typeof record.routingObservation?.modelId !== "string" ||
    !record.routingObservation.modelId.trim() ||
    !["local", "cloud"].includes(record.routingObservation?.routeClass)
  ) {
    failures.push("routing observation is incomplete");
  }
  if (
    typeof record.measurement?.collector !== "string" ||
    !record.measurement.collector.trim() ||
    !["monotonic", "wall_and_monotonic"].includes(record.measurement?.clock) ||
    !Number.isInteger(record.measurement?.monotonicSequence) ||
    record.measurement.monotonicSequence < 0
  ) {
    failures.push("measurement provenance is incomplete");
  }
  return failures;
}

export function validateTelemetryFixture(schema, record) {
  return [
    ...schemaFailures(schema, record, "resource telemetry "),
    ...validateTelemetryRecord(record),
  ];
}

export function validateHeroEvidenceSchema(schema) {
  const failures = [];
  const required = schema.required ?? [];
  const expectedRequired = [
    "schemaVersion",
    "workflowId",
    "buildBinding",
    "executed",
    "terminalState",
    "startedAt",
    "finishedAt",
    "projectId",
    "taskId",
    "taskRunId",
    "postconditions",
    "resourceTelemetryRefs",
  ];
  if (!sameMembers(required, expectedRequired) || required.includes("runId")) {
    failures.push("hero evidence schema must reuse taskRunId and reject a parallel runId");
  }
  if (schema.additionalProperties !== false || schema.properties?.executed?.const !== true) {
    failures.push("hero evidence schema must be closed and require executed=true");
  }
  if (!/^\^taskrun_/.test(schema.properties?.taskRunId?.pattern ?? "")) {
    failures.push("hero evidence taskRunId must enforce the P0 prefix");
  }
  for (const [field, prefix] of [["projectId", "project"], ["taskId", "task"]]) {
    if (!new RegExp(`^\\^${prefix}_`).test(schema.properties?.[field]?.pattern ?? "")) {
      failures.push(`hero evidence ${field} must enforce the P0 prefix`);
    }
  }
  const evidenceEnum = schema.properties?.postconditions?.items?.properties?.evidenceClass?.enum ?? [];
  if (!sameMembers(evidenceEnum, [...VALID_EVIDENCE_TYPES])) {
    failures.push("hero evidence schema must use exactly the P0 evidence classes");
  }
  const postconditions = schema.properties?.postconditions;
  const postconditionItems = postconditions?.items;
  if (
    postconditions?.minItems !== 1 ||
    postconditionItems?.additionalProperties !== false ||
    !sameMembers(postconditionItems?.required ?? [], [
      "id",
      "evidenceClass",
      "status",
      "observedAt",
      "evidenceIds",
      "details",
    ]) ||
    !sameMembers(postconditionItems?.properties?.status?.enum ?? [], ["verified", "failed", "blocked"]) ||
    postconditionItems?.properties?.evidenceIds?.minItems !== 1 ||
    schema.properties?.resourceTelemetryRefs?.minItems !== 1
  ) {
    failures.push("hero evidence schema must require complete, non-not-run postcondition evidence");
  }
  const conditionalCounts = new Map(
    (schema.allOf ?? []).map((condition) => [
      condition.if?.properties?.workflowId?.const,
      [
        condition.then?.properties?.postconditions?.minItems,
        condition.then?.properties?.postconditions?.maxItems,
      ],
    ]),
  );
  for (const [workflowId, requirement] of Object.entries(HERO_REQUIREMENTS)) {
    const expectedCount = requirement.postconditionIds.length;
    const [minimum, maximum] = conditionalCounts.get(workflowId) ?? [];
    if (minimum !== expectedCount || maximum !== expectedCount) {
      failures.push(`${workflowId}: hero evidence schema must require exactly ${expectedCount} postconditions`);
    }
  }
  return failures;
}

function schemaFailures(schema, record, label) {
  const ajv = new Ajv2020({ allErrors: true, strict: false });
  addFormats(ajv);
  const validate = ajv.compile(schema);
  if (validate(record)) return [];
  return (validate.errors ?? []).map(
    (error) => `${label}${error.instancePath || "/"}: ${error.message}`,
  );
}

function hasFieldPath(value, fieldPath) {
  let current = value;
  for (const segment of fieldPath.split(".")) {
    if (current === null || typeof current !== "object" || !(segment in current)) return false;
    current = current[segment];
  }
  return current !== undefined;
}

export function validateHeroEvidenceRecord(definitions, schema, record) {
  const failures = schemaFailures(schema, record, "hero evidence ");
  const workflow = (definitions.workflows ?? []).find((item) => item.id === record.workflowId);
  if (!workflow) return [...failures, "hero evidence references an unknown workflow"];
  const expected = new Map(workflow.postconditions.map((item) => [item.id, item]));
  const actualIds = (record.postconditions ?? []).map((item) => item.id);
  if (
    !sameMembers(actualIds, [...expected.keys()]) ||
    new Set(actualIds).size !== actualIds.length ||
    actualIds.length !== expected.size
  ) {
    failures.push("hero evidence postcondition IDs do not exactly match the workflow contract");
  }
  for (const postcondition of record.postconditions ?? []) {
    const definition = expected.get(postcondition.id);
    if (!definition) continue;
    if (postcondition.evidenceClass !== definition.evidenceClass) {
      failures.push(`${postcondition.id}: evidence class differs from the workflow contract`);
    }
    const fieldScope = { ...record, ...postcondition, ...(postcondition.details ?? {}) };
    for (const field of definition.requiredFields) {
      if (!hasFieldPath(fieldScope, field)) {
        failures.push(`${postcondition.id}: evidence is missing required field ${field}`);
      }
    }
  }
  return failures;
}

function baselineSourceFiles(root) {
  return execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { cwd: root, encoding: "utf8" },
  )
    .split("\0")
    .filter(Boolean)
    .filter((file) =>
      /^(?:src\/|src-tauri\/(?:src\/|tests\/|build\.rs$|Cargo\.(?:toml|lock)$|capabilities\/|permissions\/|tauri[^/]*\.json$)|scripts\/|schemas\/|evaluations\/|package(?:-lock)?\.json$|\.github\/workflows\/release\.yml$|docs\/architecture\/p1-contract-ownership\.md$)/.test(file),
    )
    .filter((file) => !file.startsWith("release/baselines/"))
    .filter((file) => existsSync(path.join(root, file)))
    .sort();
}

export function computeSourceTreeDigest(root = defaultRoot) {
  const digest = createHash("sha256");
  for (const file of baselineSourceFiles(root)) {
    digest.update(file);
    digest.update("\0");
    digest.update(createHash("sha256").update(readFileSync(path.join(root, file))).digest("hex"));
    digest.update("\0");
  }
  return `sha256:${digest.digest("hex")}`;
}

export const PROTECTED_HANDOFF_FILES = [
  "src/lib/p0Contracts.ts",
  "src-tauri/src/p0_contracts.rs",
  "schemas/p0-contract-v1-vectors.json",
  "scripts/p0-domain-ownership.json",
  "scripts/check-p0-architecture.mjs",
  "scripts/p0-golden-tasks.mjs",
  "scripts/check-real-components.mjs",
  "src/lib/p1Contracts.ts",
  "src-tauri/src/p1_contracts.rs",
  "schemas/p1-contract-v1-vectors.json",
  "evaluations/p1/hero-workflows.json",
  "schemas/p1-hero-workflow-evidence.schema.json",
  "schemas/p1-resource-budget-telemetry.schema.json",
];

export function computeProtectedSurfaceDigest(root = defaultRoot, contentOverrides = {}) {
  const digest = createHash("sha256");
  for (const file of PROTECTED_HANDOFF_FILES) {
    const absolute = path.join(root, file);
    if (!existsSync(absolute)) throw new Error(`protected handoff surface is missing: ${file}`);
    const content = Object.hasOwn(contentOverrides, file)
      ? contentOverrides[file]
      : readFileSync(absolute);
    digest.update(file);
    digest.update("\0");
    digest.update(createHash("sha256").update(content).digest("hex"));
    digest.update("\0");
  }
  return `sha256:${digest.digest("hex")}`;
}

export function computeWorktreeStatusDigest(root = defaultRoot) {
  const status = execFileSync("git", ["status", "--porcelain=v1", "-z"], {
    cwd: root,
    encoding: null,
  });
  return `sha256:${sha256(status)}`;
}

export function validateBaselineManifest(manifest, root = defaultRoot) {
  const failures = [];
  if (
    manifest.schemaVersion !== 1 ||
    manifest.sprint !== 233 ||
    manifest.releaseClaim !== false ||
    manifest.status !== "qualified-with-explicit-exclusions"
  ) {
    failures.push("Sprint 233 baseline must be versioned and must not claim a release");
  }
  if (
    typeof manifest.capturedAt !== "string" ||
    !manifest.capturedAt.endsWith("Z") ||
    Number.isNaN(Date.parse(manifest.capturedAt)) ||
    typeof manifest.source?.version !== "string" ||
    !manifest.source.version.trim() ||
    !/^[0-9a-f]{40}$/.test(manifest.source?.sourceRevision ?? "") ||
    !/^sha256:[0-9a-f]{64}$/.test(manifest.source?.worktreeStatusDigest ?? "") ||
    !/^sha256:[0-9a-f]{64}$/.test(manifest.source?.sourceTreeDigest ?? "")
  ) {
    failures.push("baseline source revision, version, capturedAt, and source digests are invalid");
  } else {
    try { execFileSync("git", ["cat-file", "-e", `${manifest.source.sourceRevision}^{commit}`], { cwd: root }); }
    catch { failures.push("baseline sourceRevision is not an available historical commit"); }
  }
  if (!Array.isArray(manifest.checks) || manifest.checks.length === 0) {
    failures.push("baseline must contain checks that were actually executed");
  }
  const checkIds = new Set();
  for (const check of manifest.checks ?? []) {
    if (checkIds.has(check.id)) failures.push(`${check.id}: duplicate baseline check`);
    checkIds.add(check.id);
    if (check.executed !== true || check.status !== "passed" || check.exitCode !== 0) {
      failures.push(`${check.id}: accepted baseline checks must execute, pass, and exit zero`);
    }
    if (!Array.isArray(check.command) || check.command.length < 2) {
      failures.push(`${check.id}: command and exit code are required`);
    }
    const implementationPath = check.gateImplementationPath;
    const allowedAbsolute = implementationPath === "/usr/bin/codesign";
    if (
      typeof implementationPath !== "string" ||
      (!allowedAbsolute && (path.isAbsolute(implementationPath) || implementationPath.includes("..")))
    ) {
      failures.push(`${check.id}: gate implementation path is invalid`);
      continue;
    }
    const absolute = allowedAbsolute ? implementationPath : path.join(root, implementationPath);
    if (!existsSync(absolute)) {
      failures.push(`${check.id}: gate implementation path is missing`);
      continue;
    }
    if (!/^sha256:[0-9a-f]{64}$/.test(check.gateImplementationDigest ?? "")) {
      failures.push(`${check.id}: gate implementation digest is invalid`);
      continue;
    }
    // Gate digests are immutable historical evidence from a dirty Sprint 233
    // handoff. Current or later-sprint file contents must not rewrite history.
  }
  if (!sameMembers([...checkIds], REQUIRED_BASELINE_CHECK_IDS) || checkIds.size !== (manifest.checks ?? []).length) {
    failures.push("baseline check inventory has an omission, duplicate, or unknown check");
  }
  const exclusionIds = new Set();
  for (const exclusion of manifest.exclusions ?? []) {
    if (exclusionIds.has(exclusion.id)) failures.push(`${exclusion.id}: duplicate exclusion`);
    exclusionIds.add(exclusion.id);
    if (!["environmental", "implementation"].includes(exclusion.classification)) {
      failures.push(`${exclusion.id}: exclusion classification is invalid`);
    }
    if (!['environment-blocked', 'not-run'].includes(exclusion.status) || exclusion.doesNotCountAsPass !== true) {
      failures.push(`${exclusion.id}: exclusions cannot count as passing evidence`);
    }
    if (typeof exclusion.reason !== "string" || exclusion.reason.length < 24) {
      failures.push(`${exclusion.id}: a concrete exclusion reason is required`);
    }
  }
  const environmentalIds = new Set(
    (manifest.exclusions ?? [])
      .filter((item) => item.classification === "environmental")
      .map((item) => item.id),
  );
  for (const required of ["signed-notarized-release", "fresh-install-hero", "external-channel-acceptance"]) {
    if (!environmentalIds.has(required)) failures.push(`missing environmental exclusion: ${required}`);
  }
  return failures;
}

function validateNoTrackedSprintPlans(root) {
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
    .split("\0")
    .filter(Boolean);
  return tracked.some((file) => /^planning\/sprints\//.test(file))
    ? ["planning sprint documents must not be tracked in the repository"]
    : [];
}

export function validateNoOfficeAutomation(root, sourceOverrides = {}) {
  const tracked = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { cwd: root, encoding: "utf8" },
  )
    .split("\0")
    .filter(Boolean)
    .filter((file) => /^(?:src\/|src-tauri\/(?:src\/|tests\/|build\.rs$)|scripts\/)/.test(file));
  const files = [...new Set([...tracked, ...Object.keys(sourceOverrides)])].sort();
  const forbidden = [["sof", "fice"], ["libre", "office"], ["open", "office"]]
    .map((parts) => parts.join(""));
  const qualifiedExactPackageRenderSeams = new Set([
    "src-tauri/src/artifacts/presentations/exact_package_preview.rs",
    "src-tauri/src/artifacts/presentations/exact_package_preview_tests.rs",
    "src-tauri/src/artifacts/workbooks/exact_package_preview.rs",
    "src-tauri/src/artifacts/workbooks/exact_package_process.rs",
  ]);
  return files.flatMap((file) => {
    if (qualifiedExactPackageRenderSeams.has(file)) return [];
    const absolute = path.join(root, file);
    if (!Object.hasOwn(sourceOverrides, file) && !existsSync(absolute)) return [];
    const source = String(Object.hasOwn(sourceOverrides, file)
      ? sourceOverrides[file]
      : readFileSync(absolute)).toLowerCase();
    return forbidden.some((token) => source.includes(token))
      ? [`${file}: external office discovery or invocation is forbidden`]
      : [];
  });
}

export function inspectP1ContractGate(
  root = defaultRoot,
  { allowMissingSharedFixture = false } = {},
) {
  const ownership = loadJson(root, "scripts/p1-domain-ownership.json");
  const gateManifest = loadJson(root, "scripts/p1-verification-gates.json");
  const heroes = loadJson(root, "evaluations/p1/hero-workflows.json");
  const heroEvidenceSchema = loadJson(root, "schemas/p1-hero-workflow-evidence.schema.json");
  const heroEvidence = loadJson(root, "evaluations/p1/fixtures/hero-workflow-evidence.valid.json");
  const telemetrySchema = loadJson(root, "schemas/p1-resource-budget-telemetry.schema.json");
  const telemetry = loadJson(root, "evaluations/p1/fixtures/resource-budget-telemetry.valid.json");
  const gateResult = validateGateManifest(root, gateManifest, { allowMissingSharedFixture });
  const failures = [
    ...validateOwnership(root, ownership),
    ...gateResult.failures,
    ...validateHeroDefinitions(heroes),
    ...validateHeroEvidenceSchema(heroEvidenceSchema),
    ...validateHeroEvidenceRecord(heroes, heroEvidenceSchema, heroEvidence),
    ...validateTelemetrySchema(telemetrySchema),
    ...validateTelemetryFixture(telemetrySchema, telemetry),
    ...validateNoTrackedSprintPlans(root),
    ...validateNoOfficeAutomation(root),
  ];
  return { failures, deferred: gateResult.deferred };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const allowPending = process.argv.includes("--allow-pending-shared-fixture");
  const result = inspectP1ContractGate(defaultRoot, {
    allowMissingSharedFixture: allowPending,
    allowMissingBaseline: process.argv.includes("--allow-missing-baseline"),
  });
  if (result.deferred.length > 0) {
    console.warn("p1-contract-gate: DEFERRED");
    for (const item of result.deferred) console.warn(`  - ${item}`);
  }
  if (result.failures.length > 0) {
    console.error("p1-contract-gate: FAIL");
    for (const failure of result.failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log("p1-contract-gate: PASS (ownership, ratchets, parity, heroes, telemetry, and protected surface verified)");
}
