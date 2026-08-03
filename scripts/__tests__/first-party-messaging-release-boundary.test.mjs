import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

describe("first-party messaging release boundary", () => {
  it("packages only the reviewed application executables", () => {
    const config = JSON.parse(
      readFileSync(resolve(root, "src-tauri/tauri.release.conf.json"), "utf8"),
    );
    expect(config.bundle.externalBin).toEqual([
      "binaries/local_infer",
      "binaries/pdf_extract_helper",
      "binaries/artifact_build_helper",
      "binaries/oomu-artifact-pdf-helper",
      "binaries/oomu-vision-helper",
      "binaries/oomu-speech-bridge",
    ]);
  });

  it("keeps retired messaging runtimes out of the canonical release entrypoint", () => {
    const release = readFileSync(resolve(root, "scripts/release.mjs"), "utf8");
    expect(release).not.toMatch(/embeddedSidecar|signed_sidecar_handshake|sidecar_validation/);
  });
});
