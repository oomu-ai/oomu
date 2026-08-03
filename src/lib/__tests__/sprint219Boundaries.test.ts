import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const source = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Sprint 219 trust-boundary absence checks", () => {
  it("has no privileged browser development bridge or plaintext mock store", () => {
    expect(existsSync(resolve(root, "scripts/dev.mjs"))).toBe(false);
    expect(existsSync(resolve(root, "scripts/tauri-mock/handler.mts"))).toBe(false);
    expect(existsSync(resolve(root, "app_data/tauri_mock_store.json"))).toBe(false);
    expect(existsSync(resolve(root, "scripts/tauri-mock"))).toBe(false);
    expect(existsSync(resolve(root, "src/app/api/tauri-mock"))).toBe(false);
    expect(source("package.json")).not.toContain("NEXT_PUBLIC_TAURI_MOCK_URL");
    expect(source("src/lib/invoke.ts")).not.toContain("/api/tauri-mock");
    expect(source("src/lib/invoke.ts")).not.toContain("127.0.0.1:3001");
  });

  it("requires opaque picker grants instead of HOME path membership", () => {
    const localContext = source("src-tauri/src/local_context.rs");
    expect(localContext).not.toContain("safe_roots");
    expect(localContext).not.toContain("home_dir()");
    expect(localContext).toContain("grant_id");
    expect(localContext).toContain("local_context_grant_scope_mismatch");
    expect(localContext).toContain("content_sha256");
    const vision = source("src-tauri/src/tools/vision.rs");
    expect(vision).not.toContain("fn safe_roots");
    expect(vision).not.toContain("fn guard_artifact_path");
  });

  it("does not log inbound message excerpts", () => {
    const gateway = source("src-tauri/src/gateway.rs");
    expect(gateway).not.toContain("body_preview");
    expect(gateway).toContain("body_bytes=");
    expect(gateway).toContain("correlation_hash=");
  });

  it("keeps eval harness credentials on a validated non-redirecting HTTPS endpoint", () => {
    const evaluator = source("scripts/workflow-compose-eval.mts");
    expect(evaluator).toContain('parsed.hostname === "127.0.0.1"');
    expect(evaluator).toContain('parsed.hostname === "[::1]"');
    expect(evaluator).toContain('redirect: "error"');
    expect(evaluator).toContain("endpointClass: dryRun");
    expect(evaluator).not.toContain("endpoint: dryRun ? null : endpoint,");
    expect(evaluator).toContain("redactSensitiveValue(report)");
  });

  it("stages picker visual bytes privately and never reads adjacent sidecars", () => {
    const vision = source("src-tauri/src/tools/vision.rs");
    expect(vision).toContain("create_new(true)");
    expect(vision).toContain("builder.mode(0o700)");
    expect(vision).toContain("options.mode(0o600)");
    expect(vision).not.toContain("extract_sidecar_facts");
    expect(vision).not.toContain('join("oomu-visual-artifacts")');
  });

  it("keeps localhost renderer networking development-only and ships no iframe browser fallback", () => {
    const production = JSON.parse(source("src-tauri/tauri.conf.json")) as {
      app: { security: { csp: string } };
    };
    const development = JSON.parse(source("src-tauri/tauri.development.conf.json")) as {
      app: { security: { csp: string } };
    };
    expect(production.app.security.csp).not.toContain("ws://localhost:*");
    expect(production.app.security.csp).not.toContain("http://localhost:*");
    expect(development.app.security.csp).toContain("ws://localhost:*");
    expect(development.app.security.csp).toContain("http://localhost:*");
    expect(source("src/app/components/ChatScreen.tsx")).not.toContain("<iframe");
  });
});
