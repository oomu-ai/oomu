import { describe, expect, it } from "vitest";
import {
  ATTACHMENT_LIMITS,
  processAttachmentsBounded,
  type AttachmentCandidate,
} from "../attachmentProcessing";

describe("bounded attachment processing", () => {
  it("preflights aggregate limits before starting reads", async () => {
    const started: string[] = [];
    const candidates = ["a", "b", "c"].map((name) => ({
      name,
      decodedByteCount: 8 * 1024 * 1024,
      encodedByteCount: 0,
      process: async () => {
        started.push(name);
        return name;
      },
    }));
    const results = await processAttachmentsBounded(candidates, {
      signal: new AbortController().signal,
    });
    expect(started).toEqual(["a", "b"]);
    expect(results[2]).toMatchObject({
      ok: false,
      errorCode: "attachment_aggregate_byte_limit_exceeded",
    });
  });

  it("never exceeds bounded concurrency and isolates failures", async () => {
    let active = 0;
    let peak = 0;
    const candidates: AttachmentCandidate<string>[] = Array.from({ length: 5 }, (_, index) => ({
      name: String(index),
      decodedByteCount: 1,
      encodedByteCount: 0,
      process: async () => {
        active += 1;
        peak = Math.max(peak, active);
        await Promise.resolve();
        active -= 1;
        if (index === 2) throw new Error("raw-canary-must-not-escape");
        return String(index);
      },
    }));
    const results = await processAttachmentsBounded(candidates, {
      signal: new AbortController().signal,
    });
    expect(peak).toBeLessThanOrEqual(ATTACHMENT_LIMITS.concurrency);
    expect(results.filter((result) => result.ok)).toHaveLength(4);
    expect(JSON.stringify(results)).not.toContain("raw-canary-must-not-escape");
  });

  it("cancels queued work and releases rejected buffers", async () => {
    const controller = new AbortController();
    const released: string[] = [];
    const candidates = Array.from({ length: 5 }, (_, index) => ({
      name: String(index),
      decodedByteCount: 1,
      encodedByteCount: 0,
      release: () => released.push(String(index)),
      process: async () => {
        controller.abort();
        return index;
      },
    }));
    const results = await processAttachmentsBounded(candidates, {
      signal: controller.signal,
    });
    expect(results.every((result) => !result.ok)).toBe(true);
    expect(released.length).toBeGreaterThan(0);
  });
});
