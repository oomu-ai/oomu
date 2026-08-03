import { describe, expect, it } from "vitest";
import {
  classifyRustAdvisoryReport,
  parseCargoAuditReport,
} from "../audit-rust-dependencies.mjs";

function report(findings = [], ignore = []) {
  return {
    settings: { ignore },
    vulnerabilities: { list: findings },
  };
}

function finding(id, name, version) {
  return {
    advisory: { id },
    package: { name, version },
  };
}

describe("Rust advisory release gate", () => {
  it("accepts cargo-audit JSON from stderr when findings make it exit non-zero", () => {
    const payload = JSON.stringify({ database: {}, settings: {}, vulnerabilities: {} });
    expect(parseCargoAuditReport("", payload)).toMatchObject({ database: {} });
  });

  it("accepts a clean full scan without ignores", () => {
    expect(classifyRustAdvisoryReport(report(), "")).toMatchObject({
      status: "passed",
      full_lockfile_scanned: true,
      reachable_vulnerability_count: 0,
    });
  });

  it("rejects the PDF advisory and any ignore entry", () => {
    expect(() =>
      classifyRustAdvisoryReport(
        report([finding("RUSTSEC-2026-0187", "lopdf", "0.34.0")]),
        "",
      ),
    ).toThrow(/RUSTSEC-2026-0187/u);
    expect(() => classifyRustAdvisoryReport(report([], ["RUSTSEC-1"]), "")).toThrow(
      /ignore entries/u,
    );
  });

  it("allows only the exact target-unreachable Wayland findings", () => {
    const findings = [
      finding("RUSTSEC-2026-0194", "quick-xml", "0.39.2"),
      finding("RUSTSEC-2026-0195", "quick-xml", "0.39.2"),
    ];
    expect(classifyRustAdvisoryReport(report(findings), "")).toMatchObject({
      full_lockfile_finding_count: 2,
      reachable_vulnerability_count: 0,
    });
    expect(() => classifyRustAdvisoryReport(report(findings), "quick-xml v0.39.2"))
      .toThrow(/became reachable/u);
    expect(() =>
      classifyRustAdvisoryReport(
        report([finding("RUSTSEC-2026-0194", "quick-xml", "0.39.3")]),
        "",
      ),
    ).toThrow(/Unreviewed/u);
  });
});
