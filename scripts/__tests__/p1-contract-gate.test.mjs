import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  inspectP1ContractGate,
  computeProtectedSurfaceDigest,
  validateGateManifest,
  validateHeroEvidenceRecord,
  validateHeroEvidenceSchema,
  validateHeroDefinitions,
  validateMicrosoftImplementationFixture,
  validatePresentationImplementationFixture,
  validateAppControlImplementationFixture,
  validateNoOfficeAutomation,
  validateOwnership,
  validateTelemetryFixture,
  validateTelemetrySchema,
  validateWorkbookImplementationFixture,
} from "../check-p1-contract-gate.mjs";

const root = path.resolve(import.meta.dirname, "../..");
const load = (relativePath) => JSON.parse(readFileSync(path.join(root, relativePath), "utf8"));

describe("Sprint 243 P1 contract gate", () => {
  it("implements the 234-242 roots with honest external qualification state", () => {
    const ownership = load("scripts/p1-domain-ownership.json");
    expect(validateOwnership(root, ownership)).toEqual([]);
    expect(ownership).toMatchObject({ schemaVersion: 2, contractFreezeSprint: 233, latestImplementedSprint: 243 });
    expect(ownership.domains).toHaveLength(9);
    expect(ownership.domains.every((domain) => domain.implemented && !domain.reservationOnly)).toBe(true);
    expect(ownership.domains.every((domain) => domain.qualificationStatus === "contract-verified-external-not-run")).toBe(true);

    const unbounded = structuredClone(ownership);
    unbounded.domains[0].roots[0] = "src-tauri/src";
    unbounded.domains[0].cycleNodes[0] = "rust:";
    unbounded.sharedContractFiles[0].maximumLines = 5000;
    expect(validateOwnership(root, unbounded).join("\n")).toMatch(/exceeds P1 domain ceiling|1500-line ceiling/);
  });

  it("freezes the exact protected domains and executable gate commands", () => {
    const manifest = load("scripts/p1-verification-gates.json");
    expect(validateGateManifest(root, manifest).failures).toEqual([]);
    expect(manifest).toMatchObject({ schemaVersion: 2, contractFreezeSprint: 233, latestImplementedSprint: 243 });

    const weakened = structuredClone(manifest);
    weakened.protectedP0Domains = [];
    weakened.gates[0].command = ["node", "unreviewed-gate.mjs"];
    expect(validateGateManifest(root, weakened).failures.join("\n")).toMatch(
      /exact seven P0 domains|command differs/,
    );

    const detachedPolicy = structuredClone(manifest);
    detachedPolicy.qualityPolicyPath = "release/missing-quality-policy.json";
    expect(validateGateManifest(root, detachedPolicy).failures.join("\n")).toMatch(
      /public quality-gate policy/iu,
    );
  });

  it("defines three evidence-typed hero workflows without executable behavior", () => {
    const heroes = load("evaluations/p1/hero-workflows.json");
    const evidenceSchema = load("schemas/p1-hero-workflow-evidence.schema.json");
    expect(validateHeroDefinitions(heroes)).toEqual([]);
    expect(validateHeroEvidenceSchema(evidenceSchema)).toEqual([]);

    const invalid = structuredClone(heroes);
    invalid.workflows[0].postconditions[0].evidenceClass = "unverified_claim";
    invalid.workflows[0].productionCommands = ["future_command"];
    invalid.workflows[0].implementationSprints = [];
    invalid.workflows[1].requiredContracts = ["MediaAsset", "MediaAsset", "DesktopAction"];
    invalid.workflows[2].postconditions[0].requiredFields[0] = "";
    expect(validateHeroDefinitions(invalid).join("\n")).toMatch(/invalid evidence type|cannot imply/);
  });

  it("validates a concrete hero record against Draft 2020-12 and exact workflow IDs", () => {
    const heroes = load("evaluations/p1/hero-workflows.json");
    const schema = load("schemas/p1-hero-workflow-evidence.schema.json");
    const evidence = load("evaluations/p1/fixtures/hero-workflow-evidence.valid.json");
    expect(validateHeroEvidenceRecord(heroes, schema, evidence)).toEqual([]);

    const weakenedSchema = structuredClone(schema);
    weakenedSchema.properties.executed = { type: "boolean" };
    weakenedSchema.properties.postconditions.minItems = 0;
    weakenedSchema.properties.postconditions.items.required = ["id"];
    expect(validateHeroEvidenceSchema(weakenedSchema).join("\n")).toMatch(
      /executed=true|complete, non-not-run/,
    );

    const incomplete = structuredClone(evidence);
    incomplete.postconditions[0].id = "substituted-success";
    delete incomplete.postconditions[1].details.transcript.text;
    expect(validateHeroEvidenceRecord(heroes, schema, incomplete).join("\n")).toMatch(
      /postcondition IDs|transcript.text/,
    );
  });

  it("requires all five budgets and makes route information observational only", () => {
    const schema = load("schemas/p1-resource-budget-telemetry.schema.json");
    const telemetry = load("evaluations/p1/fixtures/resource-budget-telemetry.valid.json");
    expect(validateTelemetrySchema(schema)).toEqual([]);
    expect(validateTelemetryFixture(schema, telemetry)).toEqual([]);

    const forwardCompatible = structuredClone(telemetry);
    forwardCompatible.resourceBudget.limits.futureLimit = 1;
    forwardCompatible.resourceBudget.usage.futureUsage = 0;
    expect(validateTelemetryFixture(schema, forwardCompatible)).toEqual([]);

    const invalid = structuredClone(telemetry);
    delete invalid.resourceBudget.limits.mutations;
    invalid.routingObservation.policyMutation = true;
    delete invalid.build;
    delete invalid.process;
    delete invalid.system;
    delete invalid.phase;
    invalid.sampledAt = "not-a-time";
    expect(validateTelemetryFixture(schema, invalid).join("\n")).toMatch(
      /mutations|routing policy|valid build|process|system|phase|timestamp/,
    );

    const weakenedSchema = structuredClone(schema);
    delete weakenedSchema.properties.resourceBudget.properties.limits.properties.concurrentChildren.maximum;
    weakenedSchema.properties.routingObservation.properties.policyMutation = { type: "boolean" };
    expect(validateTelemetrySchema(weakenedSchema).join("\n")).toMatch(
      /concurrency constraints|routing-policy mutation/,
    );
  });

  it("pins and protects the current P0/shared contract surface", () => {
    const manifest = load("scripts/p1-verification-gates.json");
    const digestTamper = structuredClone(manifest);
    digestTamper.protectedSurfaceDigest = `sha256:${"0".repeat(64)}`;
    expect(validateGateManifest(root, digestTamper).failures.join("\n")).toMatch(
      /protected P0\/shared contract surface has drifted/iu,
    );

    const p0Path = "src/lib/p0Contracts.ts";
    const p0Source = readFileSync(path.join(root, p0Path));
    expect(validateGateManifest(root, manifest, {
      protectedContentOverrides: { [p0Path]: Buffer.concat([p0Source, Buffer.from("\n// drift")]) },
    }).failures.join("\n")).toMatch(/protected P0\/shared contract surface has drifted/iu);

    const digest = computeProtectedSurfaceDigest(root);
    expect(computeProtectedSurfaceDigest(root, {
      "src-tauri/src/connectors/microsoft365/new-sprint-file.rs": Buffer.from("allowed"),
    })).toBe(digest);
  });

  it("strictly validates all implemented Sprint 234-237 fixtures", () => {
    const manifest = load("scripts/p1-verification-gates.json");
    const microsoftPath = manifest.implementationFixtures[0].path;
    const workbookPath = manifest.implementationFixtures[1].path;
    const presentationPath = manifest.implementationFixtures[2].path;
    const appControlPath = manifest.implementationFixtures[3].path;
    expect(validateGateManifest(root, manifest).failures).toEqual([]);

    const microsoft = load(microsoftPath);
    microsoft.accounts[0].capabilityGrants[0].remoteMutation = true;
    const workbook = load(workbookPath);
    workbook.reviews[0].calculation.engine = "invented-engine";
    const presentation = load(presentationPath);
    presentation.reviews[0].summary.exportable = false;
    const presentationBadImage = load(presentationPath);
    presentationBadImage.reviews[0].filmstrip[0].thumbnail.bytesBase64 = "iVBORw0KGgo=";
    const presentationNoExactRender = load(presentationPath);
    presentationNoExactRender.reviews[0].verification.checks = presentationNoExactRender.reviews[0].verification.checks.filter((check) => check.code !== "exact_package_pages_rendered");
    const appControl = load(appControlPath);
    appControl.evidence.staleReplayAllowed = true;

    expect(validateMicrosoftImplementationFixture(microsoft).join("\n")).toMatch(/production operation grant/);
    expect(validateWorkbookImplementationFixture(workbook).join("\n")).toMatch(/renderer fields/);
    expect(validatePresentationImplementationFixture(presentation).join("\n")).toMatch(/summary/);
    expect(validatePresentationImplementationFixture(presentationBadImage).join("\n")).toMatch(/thumbnail/);
    expect(validatePresentationImplementationFixture(presentationNoExactRender).join("\n")).toMatch(/exact-package page evidence/);
    expect(validateAppControlImplementationFixture(appControl).join("\n")).toMatch(/no stale replay/);
  });

  it("binds existing ratchets and reports only genuinely pending shared evidence", () => {
    const result = inspectP1ContractGate(root, {
      allowMissingSharedFixture: true,
      allowMissingBaseline: true,
    });
    expect(result.failures).toEqual([]);
    for (const deferred of result.deferred) {
      expect(deferred).toMatch(/P1 (contract fixture|Rust contracts|Rust parity tests)/);
    }
  });

  it("forbids external office invocation outside qualified exact-package render seams", () => {
    expect(validateNoOfficeAutomation(root)).toEqual([]);
    const token = ["sof", "fice"].join("");
    expect(validateNoOfficeAutomation(root, {
      "src-tauri/src/unsafe_office_probe.rs": Buffer.from(`Command::new("${token}")`),
    }).join("\n")).toMatch(/external office discovery or invocation is forbidden/);
  });
});
