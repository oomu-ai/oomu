import { describe, expect, it } from "vitest";
import { modTrustPresentation } from "../modTrustPresentation";

describe("modTrustPresentation", () => {
  it("promotes only a backend-reviewed mod to the OOMU reviewed state", () => {
    expect(modTrustPresentation({ reviewState: "reviewed", integrityState: "verified" }).labelKey).toBe("mods.reviewed_by_oomu");
    expect(modTrustPresentation({ reviewState: "reviewed" }).labelKey).toBe("mods.review_unknown");
    expect(modTrustPresentation({ reviewState: "unreviewed" }).labelKey).toBe("mods.not_reviewed");
  });

  it("shows Custom Mod only for an unreviewed package with a verified publisher identity", () => {
    expect(modTrustPresentation({ reviewState: "unreviewed", publisherIdentityVerified: true }).labelKey).toBe("mods.custom_mod");
    expect(modTrustPresentation({ reviewState: "unreviewed", publisherIdentityVerified: false }).labelKey).toBe("mods.not_reviewed");
  });

  it("shows Modified Mod only from the explicit modified integrity state", () => {
    expect(modTrustPresentation({ reviewState: "reviewed", integrityState: "modified" }).labelKey).toBe("mods.modified_mod");
    expect(modTrustPresentation({ reviewState: "reviewed", integrityState: "verified" }).labelKey).toBe("mods.reviewed_by_oomu");
  });

  it("keeps revoked and missing review data warm or neutral", () => {
    expect(modTrustPresentation({ reviewState: "revoked" })).toMatchObject({ tone: "warm", labelKey: "mods.review_withdrawn" });
    expect(modTrustPresentation({ reviewState: "revoked", integrityState: "modified" })).toMatchObject({ tone: "warm", labelKey: "mods.review_withdrawn" });
    expect(modTrustPresentation({})).toMatchObject({ tone: "neutral", labelKey: "mods.review_unknown" });
  });
});
