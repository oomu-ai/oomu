import { describe, expect, it } from "vitest";
import { compileLogKey, diagnosticLogKey } from "../DeveloperPanel";

describe("diagnosticLogKey", () => {
  it("uses log paths to distinguish repeated diagnostic labels", () => {
    const logs = [
      { name: "Tauri stderr log", path: "/app/src-tauri/stderr.log", sizeBytes: 0 },
      { name: "Tauri stderr log", path: "/app/stderr.log", sizeBytes: 0 },
    ];

    expect(logs.map(diagnosticLogKey)).toEqual([
      "/app/src-tauri/stderr.log",
      "/app/stderr.log",
    ]);
  });

  it("falls back to index-disambiguated keys for legacy log payloads", () => {
    const logs = [
      { name: "Tauri stderr log", sizeBytes: 0 },
      { name: "Tauri stderr log", sizeBytes: 0 },
    ];

    expect(logs.map(diagnosticLogKey)).toEqual([
      "Tauri stderr log-0-0",
      "Tauri stderr log-0-1",
    ]);
  });
});

describe("compileLogKey", () => {
  it("keeps streamed compile lines distinct across repeated phases", () => {
    const events = [
      { target: "frontend", phase: "preflight", stream: "stdout", elapsedMs: 10 },
      { target: "frontend", phase: "preflight", stream: "stdout", elapsedMs: 10 },
    ];

    expect(events.map(compileLogKey)).toEqual([
      "frontend-preflight-stdout-10-0",
      "frontend-preflight-stdout-10-1",
    ]);
  });
});
