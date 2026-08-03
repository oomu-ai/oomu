import { describe, expect, it } from "vitest";
import {
  assessMachO,
  parseBuildMetadata,
} from "../release-gates/macos-deployment-targets.mjs";

describe("macos_deployment_target release gate", () => {
  it("parses every native build-version record instead of assuming the main executable", () => {
    expect(
      parseBuildMetadata(`
Load command 9
      cmd LC_BUILD_VERSION
 platform MACOS
    minos 14.0
      sdk 26.0
`),
    ).toEqual([{ platform: "MACOS", minimumOs: "14.0", sdk: "26.0" }]);
  });

  it("preserves an incompatible measured minimum for the fail-closed validator", () => {
    const builds = parseBuildMetadata(`
Load command 9
 platform MACOS
    minos 15.2
      sdk 15.4
`);
    expect(builds[0].minimumOs).toBe("15.2");
    expect(
      assessMachO({ architectures: ["arm64"], builds, signatureStatus: 0 }),
    ).toContain("minimum_os_incompatible");
  });
});
