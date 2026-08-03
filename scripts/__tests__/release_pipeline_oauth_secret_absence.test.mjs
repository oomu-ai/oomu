import { existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { inspectSecretAbsence } from "../release-gates/oauth-secret-absence.mjs";

describe("oauth_secret_absence release gate", () => {
  it("keeps desktop OAuth inputs public and supplies every canonical public client value", () => {
    const root = resolve(import.meta.dirname, "../..");
    const buildScript = readFileSync(join(root, "src-tauri", "build.rs"), "utf8");
    const releaseEnvironment = readFileSync(join(root, "scripts", "release-environment.mjs"), "utf8");
    expect(existsSync(join(root, ".github", "workflows", "release.yml"))).toBe(false);
    expect(releaseEnvironment).toContain("OOMU_MICROSOFT_OAUTH_CLIENT_ID");
    expect(releaseEnvironment).toContain("OOMU_SLACK_OAUTH_BROKER_URL");
    expect(buildScript).not.toMatch(/rustc-env=OOMU_(?:GOOGLE|SLACK)_OAUTH_CLIENT_SECRET/);
    expect(buildScript).toContain("google_oauth_client_credential.rs");
  });

  it("finds a generated secret canary without printing or hard-coding it", () => {
    const root = mkdtempSync(join(tmpdir(), "oomu-oauth-scan-"));
    const canary = `generated-${crypto.randomUUID()}-${crypto.randomUUID()}`;
    writeFileSync(join(root, "resource.bin"), Buffer.from(`prefix:${canary}:suffix`));
    const inspection = inspectSecretAbsence(root, [canary]);
    expect(inspection.findings).toEqual([
      { path: "resource.bin", rule: "oauth_secret_canary_detected" },
    ]);
  });

  it("does not classify Google's installed-app protocol field as a confidential secret", () => {
    const root = mkdtempSync(join(tmpdir(), "oomu-google-desktop-oauth-"));
    mkdirSync(join(root, "Contents", "MacOS"), { recursive: true });
    const protocolCredential = `GOCSPX-${crypto.randomUUID().replaceAll("-", "")}`;
    writeFileSync(join(root, "Contents", "MacOS", "oomu"), protocolCredential);
    expect(inspectSecretAbsence(root, [protocolCredential]).findings).toEqual([]);
  });

  it("rejects Google's installed-app protocol field outside the main executable", () => {
    const root = mkdtempSync(join(tmpdir(), "oomu-google-desktop-oauth-resource-"));
    mkdirSync(join(root, "Contents", "Resources"), { recursive: true });
    const protocolCredential = `GOCSPX-${crypto.randomUUID().replaceAll("-", "")}`;
    writeFileSync(join(root, "Contents", "Resources", "credential.bin"), protocolCredential);
    expect(inspectSecretAbsence(root, [protocolCredential]).findings).toContainEqual({
      path: "Contents/Resources/credential.bin",
      rule: "oauth_secret_canary_detected",
    });
  });

  it("rejects private credential files even when their contents are empty", () => {
    const root = mkdtempSync(join(tmpdir(), "oomu-oauth-filename-"));
    mkdirSync(join(root, "resources"));
    writeFileSync(join(root, "resources", "client-secret.json"), "{}");
    expect(inspectSecretAbsence(root, []).findings[0].rule).toBe(
      "forbidden_private_credential_filename",
    );
  });

  it("finds a generated canary inside a compressed archive entry", () => {
    const root = mkdtempSync(join(tmpdir(), "oomu-oauth-archive-"));
    const source = join(root, "source");
    mkdirSync(source);
    const canary = `generated-${crypto.randomUUID()}-${crypto.randomUUID()}`;
    writeFileSync(join(source, "nested.txt"), canary);
    const archive = join(root, "resources.zip");
    const zipped = spawnSync("/usr/bin/zip", ["-q", archive, "nested.txt"], { cwd: source });
    expect(zipped.status).toBe(0);
    const inspection = inspectSecretAbsence(root, [canary]);
    expect(inspection.findings).toContainEqual({
      path: "resources.zip!/nested.txt",
      rule: "oauth_secret_canary_detected",
    });
    expect(inspection.archiveEntriesScanned).toBe(1);
  });
});
