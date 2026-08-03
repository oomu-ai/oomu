import { describe, expect, it } from "vitest";
import {
  composerDropPointIsInside,
  normalizeComposerDropEvent,
} from "./useNativeComposerDrop";

describe("native composer drop privacy", () => {
  it("keeps host paths out of the renderer event contract", () => {
    const normalized = normalizeComposerDropEvent({
      type: "over",
      position: { x: 250, y: 180 },
      paths: ["/Users/private/secret.txt"],
    });
    expect(normalized).toEqual({ type: "over", position: { x: 250, y: 180 } });
    expect(normalized).not.toHaveProperty("paths");
  });

  it("preserves the opaque native receipt for the exact dropped files", () => {
    const dropId = "a".repeat(64);
    expect(normalizeComposerDropEvent({
      type: "drop",
      dropId,
      position: { x: 1, y: 2 },
      paths: ["/Users/private/secret.txt"],
    })).toEqual({ type: "drop", dropId, position: { x: 1, y: 2 } });
  });

  it("uses the webview's logical drop coordinates without scaling them again", () => {
    const composerBounds = {
      left: 620,
      right: 1180,
      top: 610,
      bottom: 780,
    };

    expect(composerDropPointIsInside(composerBounds, { x: 930, y: 700 })).toBe(true);
    expect(composerDropPointIsInside(composerBounds, { x: 465, y: 350 })).toBe(false);
  });

  it("accepts a drop on the visible edge of the composer", () => {
    const composerBounds = { left: 40, right: 960, top: 640, bottom: 800 };

    expect(composerDropPointIsInside(composerBounds, { x: 960, y: 800 })).toBe(true);
  });
});
