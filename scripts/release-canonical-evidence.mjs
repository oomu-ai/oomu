import { basename } from "node:path";
import {
  createEvidenceRecord,
  finalizeEvidenceBundle,
  writeEvidenceRecord,
} from "./release-evidence.mjs";

function commonEvidence(context) {
  return {
    evidenceDir: context.evidenceDir,
    buildIdentifier: context.buildId,
    sourceRevision: context.sourceRevision,
    artifactIdentifier: context.artifactId,
    artifactDigest: context.artifactDigest,
  };
}

function materializeEvidence(common, evidenceType, producer, execution, result) {
  const record = createEvidenceRecord({
    ...common,
    evidenceType,
    producer,
    execution,
    result,
  });
  return writeEvidenceRecord(common.evidenceDir, record);
}

function materializeBuildEvidence(context, dependencies, common) {
  const { captures, combinedExecution, readJson, resolveExecutable } = dependencies;
  materializeEvidence(common, "apple_toolchain", {
    executable: context.toolchain.tools.xcodebuild.executable,
    component: "Apple Xcode toolchain",
    endpoint: context.toolchain.developer_directory,
    input: "selected macOS SDK and release tools",
  }, captures.get("apple_toolchain").execution, context.toolchain);
  materializeEvidence(common, "source_provenance", {
    executable: resolveExecutable("git"),
    component: "authorized clean source and dependency locks",
    endpoint: context.repositoryRoot,
    input: context.sourceRevision,
  }, combinedExecution(["source_revision", "source_dirty_state"], "git"), {
    source_revision: context.sourceRevision,
    clean: true,
    ...context.dependencyLockDigests,
  });
  materializeEvidence(common, "dependency_audit", {
    executable: context.npm,
    component: "npm production dependency graph",
    endpoint: "npm audit advisory service",
    input: "package-lock.json --omit=dev",
  }, captures.get("dependency_audit").execution,
  context.dependencyResult.metadata ?? context.dependencyResult);
  materializeEvidence(common, "pdf_containment", {
    executable: context.node,
    component: "contained lopdf helper, Rust advisory graph, and deterministic hostile PDF corpus",
    endpoint: "local real helper processes",
    input: "src-tauri/Cargo.lock and src-tauri/tests/pdf_containment.rs",
  }, combinedExecution(["rust_dependency_audit", "pdf_containment"], context.node), {
    parser: context.pdfContainmentResult.parser,
    advisory: context.pdfContainmentResult.advisory,
    cargo_audit: context.rustDependencyAuditResult,
    helper_protocol_version: context.pdfContainmentResult.helper_protocol_version,
    corpus_origin: context.pdfContainmentResult.corpus_origin,
    measurements: context.pdfContainmentResult.measurements,
  });
  materializeEvidence(common, "automated_tests", {
    executable: context.npm,
    component: "strict source quality, architecture, contracts, frontend, release-integrity, TypeScript, and Rust verification",
    endpoint: "local npm, Vitest, Node test, TypeScript, and Cargo runners",
    input: `exact source ${context.sourceRevision} and release candidate ${context.artifactId}`,
  }, combinedExecution([
    "automated_strict_lint",
    "automated_source_size",
    "automated_real_components",
    "automated_p0_architecture",
    "automated_p1_contracts",
    "automated_novice_ui",
    "automated_module_cycles",
    "automated_unused_exports",
    "automated_repository_hygiene",
    "automated_native_path_remap",
    "automated_release_integrity",
    "automated_i18n",
    "automated_typecheck",
    "automated_frontend",
    "automated_cargo_check",
    "automated_cargo_test",
  ], context.npm), {
    passed: true,
    source_revision: context.sourceRevision,
    native_path_remap: context.nativePathRemap,
    source_line_baseline_sha256: context.dependencyLockDigests.source_line_baseline_sha256,
    lint_warnings: 0,
    source_size_violations: 0,
    suites: [
      "lint", "check:source-size", "check:real-components", "check:p0-architecture",
      "check:p1-contracts", "check:novice-ui", "check:module-cycles",
      "check:unused-exports", "check:repository-hygiene", "native path-remap preflight",
      "test:release-integrity",
      "check:i18n", "typecheck", "test:frontend", "cargo check", "cargo test",
    ],
  });
  materializeEvidence(common, "release_extension_gates", {
    executable: context.node,
    component: "lexically discovered release extension gates",
    endpoint: context.stagedApp,
    input: "exact signed and stapled candidate application",
  }, captures.get("release_extension_gates").execution, context.extensionGates);
  materializeEvidence(common, "release_sanitizer", {
    executable: context.node,
    component: "general release sanitizer",
    endpoint: context.appPath,
    input: "explicit sanitizer policy over pre-sign and final assembled app",
  }, combinedExecution(["release_sanitizer_initial", "release_sanitizer_final"], context.node), {
    initial: readJson(context.sanitizerInitialPath, "Initial release sanitizer report"),
    final: readJson(context.sanitizerRawPath, "Final release sanitizer report"),
  });
}

function materializeCandidateEvidence(context, dependencies, common) {
  const { captures, combinedExecution, readJson, expectedReleaseTarget } = dependencies;
  materializeEvidence(common, "database_sanitizer", {
    executable: context.node,
    component: "release database sanitizer",
    endpoint: context.appPath,
    input: "pre-sign and final assembled app bundle",
  }, combinedExecution(["database_sanitizer_initial", "database_sanitizer_final"], context.node), {
    initial: readJson(context.databaseInitialPath, "Initial database sanitizer report"),
    final: readJson(context.databaseRawPath, "Final database sanitizer report"),
  });
  materializeEvidence(common, "entitlement_snapshot", {
    executable: context.codesign,
    component: "final signed app entitlement extraction",
    endpoint: context.candidateDir,
    input: "reviewed application entitlement snapshot",
  }, combinedExecution(
    ["entitlement_snapshot", "final_entitlement_snapshot"], context.codesign),
  context.finalEntitlementSnapshot);
  const firstSignedRelease =
    context.permissionContinuitySnapshot.continuity_review?.first_signed_release;
  materializeEvidence(common, "macos_permission_continuity", {
    executable: context.node,
    component: "signed macOS permission identity continuity gate",
    endpoint: context.stagedApp,
    input: firstSignedRelease
      ? "explicitly declared first signed release"
      : "previous signed OOMU application and current signed candidate",
  }, firstSignedRelease
    ? captures.get("permission_identity_current").execution
    : combinedExecution(["permission_identity_previous", "permission_continuity"], context.node),
  context.permissionContinuitySnapshot);
  materializeEvidence(common, "artifact_validation", {
    executable: context.toolchain.tools.assetutil.executable,
    component: "staged macOS candidate",
    endpoint: context.candidateDir,
    input: `${basename(context.stagedApp)} and ${basename(context.stagedDmg)}`,
  }, combinedExecution([
    "asset_catalog_validation", "gatekeeper_app", "verify_staged_codesign",
    "architecture_validation", "manifest_verification_final",
    "verify_immutable_candidate_codesign", "verify_immutable_dmg_codesign",
    "verify_immutable_app_ticket", "verify_immutable_dmg_ticket",
  ], context.toolchain.tools.assetutil.executable), {
    assets: context.assetResult,
    architectures: context.architecture,
    expected_target: expectedReleaseTarget,
    entry_count: context.manifest.entry_count,
    immutable_tree: true,
    final_dmg_sha256: context.dmgSha256,
    gatekeeper_assessment: "passed",
  });
  materializeEvidence(common, "signing", {
    executable: context.codesign,
    component: "OOMU app, nested code, and DMG",
    endpoint: context.candidateDir,
    input: "Developer ID Application identity and reviewed entitlement profiles",
  }, combinedExecution([
    "signing_preflight", ...context.nestedSigningLabels, "codesign_app",
    "codesign_verify_app", "codesign_details", "codesign_dmg",
    "verify_staged_codesign", "final_entitlement_snapshot",
    "verify_immutable_candidate_codesign", "verify_immutable_dmg_codesign",
  ], context.codesign), {
    ...context.signedDetails,
    application_entitlements_sha256:
      context.finalEntitlementSnapshot.application.extracted.canonical_sha256,
  });
}

function materializeDistributionEvidence(context, dependencies, common) {
  const { captures, combinedExecution } = dependencies;
  materializeEvidence(common, "notarization", {
    executable: context.toolchain.tools.notarytool.executable,
    component: "OOMU app and DMG",
    endpoint: "Apple notary service",
    input: context.artifactId,
  }, combinedExecution(
    ["notarize_app", "notarize_dmg"], context.toolchain.tools.notarytool.executable), {
    app: context.appNotaryResult,
    dmg: context.dmgNotaryResult,
  });
  materializeEvidence(common, "stapling", {
    executable: context.toolchain.tools.stapler.executable,
    component: "OOMU app and DMG tickets",
    endpoint: context.candidateDir,
    input: "accepted Apple notarization submissions",
  }, combinedExecution([
    "staple_app", "validate_staple_app", "staple_dmg", "validate_staple_dmg",
    "verify_staged_app_ticket", "verify_staged_dmg_ticket",
    "verify_immutable_app_ticket", "verify_immutable_dmg_ticket",
  ], context.toolchain.tools.stapler.executable), {
    app_ticket: "validated",
    dmg_ticket: "validated",
  });
  materializeEvidence(common, "manifest_verification", {
    executable: context.node,
    component: "exact release-tree manifest",
    endpoint: context.manifestPath,
    input: context.candidateDir,
  }, combinedExecution(
    ["manifest_generation", "manifest_verification", "manifest_verification_final"],
    context.node,
  ), {
    entry_count: context.manifest.entry_count,
    payload_sha256: context.manifest.payload_sha256,
    signature: context.manifest.signature,
    artifact_digest: context.artifactDigest,
    final_dmg_sha256: context.dmgSha256,
  });
  const candidateIntegrityRecordPath = materializeEvidence(
    common,
    "release_candidate_integrity",
    {
      executable: context.node,
      component: "inside-out code signature, entitlement, ticket, Gatekeeper, and immutable candidate verifier",
      endpoint: context.candidateDir,
      input: `${basename(context.stagedApp)} and ${basename(context.stagedDmg)}`,
    },
    captures.get("release_candidate_integrity").execution,
    context.candidateIntegrity,
  );
  const acceptanceExecution = combinedExecution(
    ["p0_acceptance_external", "p0_acceptance_validation"], context.p0AcceptanceRunner);
  materializeEvidence(common, "golden_task_matrix", {
    executable: context.p0AcceptanceRunner,
    component: "ten current-build golden tasks",
    endpoint: context.cleanMachine.machine_identifier,
    input: context.cleanMachineDmg,
  }, acceptanceExecution, context.goldenTaskMatrix);
  materializeEvidence(common, "recovery_matrix", {
    executable: context.p0AcceptanceRunner,
    component: "P0 restart and failure recovery matrix",
    endpoint: context.cleanMachine.machine_identifier,
    input: context.cleanMachineDmg,
  }, acceptanceExecution, context.recoveryMatrix);
  materializeEvidence(common, "hero_workflow", {
    executable: context.p0AcceptanceRunner,
    component: "weekly decision-brief hero workflow",
    endpoint: context.cleanMachine.machine_identifier,
    input: context.cleanMachineDmg,
  }, acceptanceExecution, context.heroWorkflowEvidence);
  materializeEvidence(common, "privacy_declarations", {
    executable: context.p0AcceptanceRunner,
    component: "binary-matched privacy and legal declaration audit",
    endpoint: context.cleanMachine.machine_identifier,
    input: context.cleanMachineDmg,
  }, acceptanceExecution, context.privacyDeclarations);
  materializeEvidence(common, "clean_machine_launch", {
    executable: context.cleanMachineRunner,
    component: "clean macOS install and launch",
    endpoint: context.cleanMachine.endpoint ?? context.cleanMachine.machine_identifier,
    input: context.cleanMachineDmg,
  }, captures.get("clean_machine_launch").execution, context.cleanMachine);
  return candidateIntegrityRecordPath;
}

export function materializeCanonicalReleaseEvidence(context, dependencies) {
  const common = commonEvidence(context);
  materializeBuildEvidence(context, dependencies, common);
  materializeCandidateEvidence(context, dependencies, common);
  const candidateIntegrityRecordPath = materializeDistributionEvidence(
    context, dependencies, common,
  );
  finalizeEvidenceBundle({ ...common, privateKeyPath: context.privateKeyPath });
  return { candidateIntegrityRecordPath };
}
