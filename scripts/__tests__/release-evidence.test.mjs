import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  REQUIRED_EVIDENCE_TYPES,
  createEvidenceRecord,
  validateEvidenceBundle,
  writeEvidenceRecord,
} from "../release-evidence.mjs";

describe("executed release evidence contract", () => {
  let evidenceDir;
  const expected = {
    buildIdentifier: "build-214-evidence",
    sourceRevision: "a".repeat(40),
    artifactIdentifier: "oomu-evidence-artifact",
    artifactDigest: `sha256:${"b".repeat(64)}`,
  };

  beforeEach(() => {
    evidenceDir = mkdtempSync(join(tmpdir(), "oomu-evidence-test-"));
  });
  afterEach(() => rmSync(evidenceDir, { recursive: true, force: true }));

  function populate(producedAt = new Date()) {
    for (const evidenceType of REQUIRED_EVIDENCE_TYPES) {
      writeEvidenceRecord(
        evidenceDir,
        createEvidenceRecord({
          evidenceType,
          ...expected,
          producer: {
            executable: "/usr/bin/true",
            component: evidenceType,
            endpoint: "local executed endpoint",
            input: "exact release input",
          },
          execution: { executed: true, exit_code: 0 },
          result: { passed: true },
          producedAt,
        }),
      );
    }
  }

  function rewrite(type, mutator) {
    const path = join(evidenceDir, `${type}.json`);
    chmodSync(path, 0o644);
    const record = JSON.parse(readFileSync(path, "utf8"));
    mutator(record);
    writeFileSync(path, `${JSON.stringify(record, null, 2)}\n`);
    chmodSync(path, 0o444);
  }

  it("accepts only a complete exact-build evidence set", () => {
    populate();
    const gate = validateEvidenceBundle({ evidenceDir, ...expected });
    expect(gate.checks).toHaveLength(REQUIRED_EVIDENCE_TYPES.length);
    expect(gate.status).toBe("passed");
  });

  it("rejects synthetic and prior-build records", () => {
    populate();
    rewrite("automated_tests", (record) => {
      record.synthetic = true;
    });
    expect(() => validateEvidenceBundle({ evidenceDir, ...expected })).toThrow(/synthetic/i);

    populate();
    rewrite("release_sanitizer", (record) => {
      record.build_identifier = "prior-build";
    });
    expect(() => validateEvidenceBundle({ evidenceDir, ...expected })).toThrow(/another build/i);
  });

  it("rejects stale records and never extends a near-expiry record", () => {
    const now = new Date("2026-07-09T12:00:00.000Z");
    populate(new Date(now.getTime() - 23 * 60 * 60 * 1000 - 59 * 60 * 1000));
    // The clean-machine record has a seven-day rule and is not the minimum.
    const gate = validateEvidenceBundle({ evidenceDir, ...expected, now });
    expect(Date.parse(gate.expires_at) - now.getTime()).toBe(60 * 1000);

    expect(() =>
      validateEvidenceBundle({
        evidenceDir,
        ...expected,
        now: new Date(now.getTime() + 61 * 1000),
      }),
    ).toThrow(/stale/i);
  });

  it("requires immutable evidence files", () => {
    populate();
    chmodSync(join(evidenceDir, "signing.json"), 0o644);
    expect(() => validateEvidenceBundle({ evidenceDir, ...expected })).toThrow(/writable/i);
  });
});
