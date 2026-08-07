import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  PATCHED_NEXT_ADVISORIES,
  assertProductionAudit,
  verifyStaticExport,
} from "../next-security-upgrade-contract.mjs";

const root = resolve(import.meta.dirname, "../..");

describe("next_security_upgrade release contract", () => {
  it("keeps Next and its ESLint config on the same patched release", () => {
    const manifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
    expect(manifest.dependencies.next).toBe("16.2.12");
    expect(manifest.devDependencies["eslint-config-next"]).toBe("16.2.12");
    expect(manifest.overrides).toEqual({
      nanoid: "3.3.17",
      postcss: "8.5.23",
      next: {
        sharp: "0.35.0",
      },
    });
    expect(PATCHED_NEXT_ADVISORIES).toEqual([
      "GHSA-8h8q-6873-q5fj",
      "GHSA-26hh-7cqf-hhc6",
      "GHSA-6gpp-xcg3-4w24",
      "GHSA-m99w-x7hq-7vfj",
      "GHSA-89xv-2m56-2m9x",
      "GHSA-68g3-v927-f742",
      "GHSA-4633-3j49-mh5q",
      "GHSA-qx2v-qp2m-jg93",
      "GHSA-6g55-p6wh-862q",
      "GHSA-r28c-9q8g-f849",
      "GHSA-f88m-g3jw-g9cj",
    ]);
  });

  it("blocks a high or critical production advisory", () => {
    expect(() => assertProductionAudit({
      metadata: { vulnerabilities: { low: 0, moderate: 0, high: 1, critical: 0 } },
    })).toThrow(/blocked/u);
  });

  it("keeps export and CSP contracts fail closed", () => {
    const nextConfig = readFileSync(resolve(root, "next.config.ts"), "utf8");
    const tauriConfig = readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8");
    const release = readFileSync(resolve(root, "scripts/release.mjs"), "utf8");
    expect(nextConfig).toContain('output: "export"');
    expect(tauriConfig).not.toMatch(/script-src[^;]*unsafe-(?:eval|inline)/u);
    expect(release).toContain('["audit", "--omit=dev", "--audit-level=high", "--json"]');
  });

  it("validates the complete built export when present", () => {
    const output = resolve(root, "out");
    if (existsSync(output)) {
      expect(verifyStaticExport(output)).toMatchObject({ htmlCount: expect.any(Number) });
    } else {
      expect(() => verifyStaticExport(output)).toThrow(/index\.html/u);
    }
  });
});
