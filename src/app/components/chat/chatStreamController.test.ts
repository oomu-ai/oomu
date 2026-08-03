import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ChatStreamController,
  chatStreamResponseMatches,
  type ChatTokenEvent,
  type ChatValidatedStreamCompleteEvent,
} from "./chatStreamController";

const tauriEvents = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauriEvents.listen }));

beforeEach(() => {
  tauriEvents.listen.mockReset();
});

const directive = [
  "<OomuSplitView>",
  "<mod_id>ai.eldris.mods.browser</mod_id>",
  "<action>NAVIGATE</action>",
  "<url>https://example.com</url>",
  "</OomuSplitView>",
].join("");

function event(overrides: Partial<ChatTokenEvent> = {}): ChatTokenEvent {
  return {
    stream_id: "stream-1",
    session_id: "session-1",
    turn_id: "turn-1",
    generation_token: "generation-1",
    sequence: 1,
    token: "Hello",
    elapsed_ms: 10,
    delivery_state: "validated",
    ...overrides,
  };
}

async function sha256(text: string) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("");
}

async function terminal(
  text: string,
  overrides: Partial<ChatValidatedStreamCompleteEvent> = {},
): Promise<ChatValidatedStreamCompleteEvent> {
  return {
    stream_id: "stream-1",
    session_id: "session-1",
    turn_id: "turn-1",
    generation_token: "generation-1",
    last_sequence: 1,
    chunk_count: 1,
    text_sha256: await sha256(text),
    delivery_state: "validated",
    ...overrides,
  };
}

function frameScheduler(frameIntervalMs = 20) {
  let nextFrame = 1;
  let frameCount = 0;
  const callbacks = new Map<number, FrameRequestCallback>();
  return {
    scheduler: {
      request(callback: FrameRequestCallback) {
        const frame = nextFrame++;
        callbacks.set(frame, callback);
        return frame;
      },
      cancel(frame: number) {
        callbacks.delete(frame);
      },
    },
    pendingCount: () => callbacks.size,
    frameCount: () => frameCount,
    flush() {
      const scheduled = [...callbacks.entries()];
      callbacks.clear();
      frameCount += scheduled.length;
      for (const [, callback] of scheduled) callback(frameCount * frameIntervalMs);
    },
  };
}

function controller(options?: {
  ownsTurn?: () => boolean;
  frameIntervalMs?: number;
  requiresNativeReceipt?: boolean;
}) {
  const visible: string[] = [];
  const directives: string[] = [];
  let firstTokens = 0;
  const frames = frameScheduler(options?.frameIntervalMs);
  const stream = new ChatStreamController({
    identity: {
      streamId: "stream-1",
      sessionId: "session-1",
      turnId: "turn-1",
      generationToken: "generation-1",
    },
    ownsTurn: options?.ownsTurn ?? (() => true),
    requiresNativeReceipt: options?.requiresNativeReceipt ?? false,
    onVisibleText: (chunk) => visible.push(chunk),
    onDirectiveSnapshot: (snapshot) => {
      directives.push(snapshot);
      return true;
    },
    onFirstToken: () => {
      firstTokens += 1;
    },
    scheduler: frames.scheduler,
  });
  return {
    stream,
    frames,
    visible,
    directives,
    firstTokenCount: () => firstTokens,
  };
}

describe("validated chat stream delivery", () => {
  it("accepts only the exact identity and contiguous validated sequence", () => {
    const state = controller();

    expect(state.stream.accept(event({ stream_id: "other" }))).toBe(false);
    expect(state.stream.accept(event({ session_id: "other" }))).toBe(false);
    expect(state.stream.accept(event({ turn_id: "other" }))).toBe(false);
    expect(state.stream.accept(event({ generation_token: "other" }))).toBe(false);
    expect(state.stream.accept(event({ token: "" }))).toBe(false);
    expect(state.stream.accept(event({ sequence: 2 }))).toBe(false);
    expect(state.stream.accept(event())).toBe(true);
    expect(state.stream.accept(event({ token: "duplicate" }))).toBe(false);
    expect(state.stream.accept(event({ sequence: 2, token: " world" }))).toBe(true);
    expect(state.firstTokenCount()).toBe(1);
  });

  it("drains a burst across separate animation frames with exact concatenation", () => {
    const state = controller();
    state.stream.accept(event({ token: "Accepted " }));
    state.stream.accept(event({ sequence: 2, token: "validated " }));
    state.stream.accept(event({ sequence: 3, token: "response." }));

    expect(state.frames.pendingCount()).toBe(1);
    state.frames.flush();
    expect(state.visible.join("")).toBe("Accepted ");
    state.frames.flush();
    expect(state.visible.join("")).toBe("Accepted validated ");
    state.frames.flush();
    expect(state.visible.join("")).toBe("Accepted validated response.");
    expect(state.frames.frameCount()).toBe(3);
  });

  it("waits for a delayed terminal receipt and the final painted frame", async () => {
    const state = controller();
    const expected = "Accepted response.";
    state.stream.accept(event({ token: "Accepted " }));
    state.stream.accept(event({ sequence: 2, token: "response." }));
    const drained = state.stream.awaitValidatedDrain(expected, 1_000);

    while (state.frames.pendingCount()) state.frames.flush();
    expect(state.visible.join("")).toBe(expected);
    expect(state.stream.acceptTerminal(await terminal(expected, {
      last_sequence: 2,
      chunk_count: 2,
    }))).toBe(true);
    await expect(drained).resolves.toBe(true);
  });

  it("ignores a wrong terminal identity and accepts the later exact receipt", async () => {
    const state = controller();
    state.stream.accept(event());
    state.frames.flush();
    const drained = state.stream.awaitValidatedDrain("Hello", 1_000);

    expect(state.stream.acceptTerminal(await terminal("Hello", {
      stream_id: "wrong-stream",
    }))).toBe(false);
    expect(state.stream.acceptTerminal(await terminal("Hello"))).toBe(true);
    await expect(drained).resolves.toBe(true);
  });

  it("rejects terminal count and last-sequence inconsistencies", async () => {
    const state = controller();
    state.stream.accept(event());

    expect(state.stream.acceptTerminal(await terminal("Hello", {
      last_sequence: 2,
    }))).toBe(false);
    expect(state.stream.acceptTerminal(await terminal("Hello", {
      chunk_count: 2,
    }))).toBe(false);
    expect(state.stream.acceptTerminal(await terminal("Hello"))).toBe(true);
  });

  it("fails closed when the terminal digest does not match the exact response", async () => {
    const state = controller();
    state.stream.accept(event());
    state.frames.flush();
    const drained = state.stream.awaitValidatedDrain("Hello", 1_000);

    expect(state.stream.acceptTerminal(await terminal("Hello", {
      text_sha256: "0".repeat(64),
    }))).toBe(true);
    await expect(drained).resolves.toBe(false);
  });

  it("falls back promptly when no terminal receipt arrives", async () => {
    const state = controller();
    state.stream.accept(event());
    state.frames.flush();

    await expect(state.stream.awaitValidatedDrain("Hello", 5)).resolves.toBe(false);
    expect(state.visible).toEqual(["Hello"]);
  });

  it("rejects duplicate terminals and every post-terminal token", async () => {
    const state = controller();
    state.stream.accept(event());
    const receipt = await terminal("Hello");

    expect(state.stream.acceptTerminal(receipt)).toBe(true);
    expect(state.stream.acceptTerminal(receipt)).toBe(false);
    expect(state.stream.accept(event({ sequence: 2, token: " late" }))).toBe(false);
  });
});

describe("validated stream visible cadence", () => {
  it.each([[60, 1], [120, 1], [60, 44], [120, 44]])(
    "paces a 50-chunk terminal burst after %i Hz has painted %i chunks",
    async (hz, prepaintedChunks) => {
      const frameIntervalMs = 1_000 / hz;
      const state = controller({ frameIntervalMs });
      const chunks = Array.from({ length: 50 }, (_, index) => `chunk-${index + 1} `);
      const expected = chunks.join("");
      chunks.forEach((token, index) => state.stream.accept(event({ sequence: index + 1, token })));
      while (state.visible.length < prepaintedChunks) state.frames.flush();
      expect(state.visible).toEqual(chunks.slice(0, prepaintedChunks));
      const frameCountAtTerminal = state.frames.frameCount();
      expect(state.stream.acceptTerminal(await terminal(expected, {
        last_sequence: chunks.length, chunk_count: chunks.length,
      }))).toBe(true);
      const drained = state.stream.awaitValidatedDrain(expected, 1_000);
      const projections = [state.visible.join("")];
      while (state.frames.pendingCount()) {
        const priorLength = state.visible.length;
        state.frames.flush();
        if (state.visible.length > priorLength) projections.push(state.visible.join(""));
      }
      expect(projections).toHaveLength(chunks.length - prepaintedChunks + 1);
      expect(projections.every((text, index) => expected.startsWith(text)
        && (index === 0 || text.length > projections[index - 1].length))).toBe(true);
      expect(projections.at(-1)).toBe(expected);
      const tailDurationMs = (state.frames.frameCount() - frameCountAtTerminal) * frameIntervalMs;
      const minimumTailDurationMs = Math.min(3_000, (chunks.length - prepaintedChunks) * 250);
      expect(tailDurationMs).toBeGreaterThanOrEqual(minimumTailDurationMs);
      expect(tailDurationMs).toBeLessThan(minimumTailDurationMs + 300);
      await expect(drained).resolves.toBe(true);
    },
  );

  it("keeps an already-visible live stream at its low-latency cadence", async () => {
    const state = controller({ frameIntervalMs: 20 });
    const expected = "First second third.";
    state.stream.accept(event({ token: "First " }));
    state.frames.flush();
    state.stream.accept(event({ sequence: 2, token: "second " }));
    state.stream.accept(event({ sequence: 3, token: "third." }));
    expect(state.stream.acceptTerminal(await terminal(expected, {
      last_sequence: 3, chunk_count: 3,
    }))).toBe(true);
    const drained = state.stream.awaitValidatedDrain(expected, 1_000);
    state.frames.flush();
    state.frames.flush();
    expect(state.visible.join("")).toBe(expected);
    expect(state.frames.frameCount()).toBe(3);
    await expect(drained).resolves.toBe(true);
  });
});

describe("validated stream arrival ordering", () => {
  it("keeps the bounded waiter alive when the response arrives before its first event", async () => {
    const state = controller();
    const expected = "Accepted response.";
    const drained = state.stream.awaitValidatedDrain(expected, 1_000);
    let settled = false;
    void drained.then(() => { settled = true; });
    await Promise.resolve();
    expect(settled).toBe(false);

    expect(state.stream.accept(event({ token: "Accepted " }))).toBe(true);
    expect(state.stream.accept(event({ sequence: 2, token: "response." }))).toBe(true);
    expect(state.stream.acceptTerminal(await terminal(expected, {
      last_sequence: 2,
      chunk_count: 2,
    }))).toBe(true);
    state.frames.flush();
    expect(state.visible.join("")).toBe("Accepted ");
    while (state.frames.pendingCount()) state.frames.flush();
    await expect(drained).resolves.toBe(true);
  });

  it("buffers an early terminal until every preceding validated token is contiguous", async () => {
    const state = controller();
    const expected = "Accepted response.";
    const drained = state.stream.awaitValidatedDrain(expected, 1_000);

    expect(state.stream.acceptTerminal(await terminal(expected, {
      last_sequence: 2,
      chunk_count: 2,
    }))).toBe(true);
    expect(state.stream.accept(event({ token: "Accepted " }))).toBe(true);
    expect(state.stream.accept(event({ sequence: 3, token: "gap" }))).toBe(false);
    expect(state.stream.accept(event({ sequence: 2, token: "response." }))).toBe(true);
    expect(state.stream.accept(event({ sequence: 3, token: "late" }))).toBe(false);

    while (state.frames.pendingCount()) state.frames.flush();
    await expect(drained).resolves.toBe(true);
  });
});

describe("validated stream reviewed boundaries", () => {
  it("retains receipt-required text while preserving validated directive projection", async () => {
    const state = controller({ requiresNativeReceipt: true });
    const expected = `Visible only after receipt.\n${directive}`;
    expect(state.stream.accept(event({ token: expected }))).toBe(true);
    const drained = state.stream.awaitValidatedDrain(expected, 1_000);

    state.frames.flush();
    expect(state.visible).toEqual([]);
    expect(state.directives).toEqual([directive]);
    expect(state.stream.acceptTerminal(await terminal(expected))).toBe(true);
    await expect(drained).resolves.toBe(true);
    expect(state.visible).toEqual([]);
  });

  it("keeps pending and final terminal transitions inside one monotonic deadline", async () => {
    const expected = "Accepted response.";
    const receipt = await terminal(expected, { last_sequence: 2, chunk_count: 2 });
    vi.useFakeTimers();
    try {
      const state = controller();
      let outcome: boolean | undefined;
      void state.stream.awaitValidatedDrain(expected, 100).then((value) => { outcome = value; });
      await vi.advanceTimersByTimeAsync(60);
      expect(state.stream.acceptTerminal(receipt)).toBe(true);
      await vi.advanceTimersByTimeAsync(30);
      expect(state.stream.accept(event({ token: "Accepted " }))).toBe(true);
      expect(state.stream.accept(event({ sequence: 2, token: "response." }))).toBe(true);
      await vi.advanceTimersByTimeAsync(11);
      expect(outcome).toBe(false);
      state.stream.teardown();
    } finally {
      vi.useRealTimers();
    }
  });

  it("fails a deferred digest reconciliation after turn ownership is lost", async () => {
    const expected = "Hello";
    const receipt = await terminal(expected);
    const digestBytes = await crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(expected),
    );
    let resolveDigest!: (value: ArrayBuffer) => void;
    const deferredDigest = new Promise<ArrayBuffer>((resolve) => { resolveDigest = resolve; });
    const digestSpy = vi.spyOn(crypto.subtle, "digest").mockReturnValueOnce(deferredDigest);
    let ownsTurn = true;
    try {
      const state = controller({ ownsTurn: () => ownsTurn });
      expect(state.stream.accept(event())).toBe(true);
      state.frames.flush();
      const drained = state.stream.awaitValidatedDrain(expected, 1_000);
      expect(state.stream.acceptTerminal(receipt)).toBe(true);
      ownsTurn = false;
      resolveDigest(digestBytes);
      await expect(drained).resolves.toBe(false);
    } finally {
      digestSpy.mockRestore();
    }
  });
});

describe("validated stream security boundaries", () => {
  it("subscribes to token and terminal events", async () => {
    const state = controller();
    const tokenUnlisten = vi.fn();
    const terminalUnlisten = vi.fn();
    tauriEvents.listen
      .mockResolvedValueOnce(tokenUnlisten)
      .mockResolvedValueOnce(terminalUnlisten);

    await expect(state.stream.listen()).resolves.toBe(true);
    expect(tauriEvents.listen.mock.calls.map(([channel]) => channel)).toEqual([
      "chat://token",
      "chat://validated-stream-complete",
    ]);
    state.stream.teardown();
    expect(tokenUnlisten).toHaveBeenCalledOnce();
    expect(terminalUnlisten).toHaveBeenCalledOnce();
  });

  it("fails drain immediately when terminal listener setup fails", async () => {
    const state = controller();
    const tokenUnlisten = vi.fn();
    tauriEvents.listen
      .mockResolvedValueOnce(tokenUnlisten)
      .mockRejectedValueOnce(new Error("terminal listener unavailable"));

    await expect(state.stream.listen()).resolves.toBe(false);
    expect(tokenUnlisten).toHaveBeenCalledOnce();
    await expect(state.stream.awaitValidatedDrain("Hello", 15_000)).resolves.toBe(false);
  });

  it("counts provisional progress while keeping text and directives invisible", () => {
    const state = controller();
    expect(state.stream.accept(event({
      token: `Untrusted ${directive}`,
      delivery_state: "provisional",
    }))).toBe(true);

    state.frames.flush();
    expect(state.firstTokenCount()).toBe(1);
    expect(state.visible).toEqual([]);
    expect(state.directives).toEqual([]);
  });

  it("activates a directive only after its validated token is painted", async () => {
    const state = controller();
    state.stream.accept(event({ token: directive }));
    expect(state.directives).toEqual([]);

    state.frames.flush();
    expect(state.visible).toEqual([]);
    expect(state.directives).toEqual([directive]);
    expect(state.stream.acceptTerminal(await terminal(directive))).toBe(true);
  });

  it("drops queued text after ownership loss", () => {
    let ownsTurn = true;
    const state = controller({ ownsTurn: () => ownsTurn });
    state.stream.accept(event());

    ownsTurn = false;
    state.frames.flush();
    expect(state.visible).toEqual([]);
    expect(state.stream.accept(event({ sequence: 2, token: "late" }))).toBe(false);
  });

  it("teardown cancels queued paints, fails the drain, and releases listeners once", async () => {
    const state = controller();
    let unlistenCalls = 0;
    state.stream.bindUnlisten(() => { unlistenCalls += 1; });
    state.stream.bindUnlisten(() => { unlistenCalls += 1; });
    state.stream.accept(event());
    const drained = state.stream.awaitValidatedDrain("Hello", 1_000);

    state.stream.teardown();
    state.stream.teardown();
    expect(unlistenCalls).toBe(2);
    expect(state.frames.pendingCount()).toBe(0);
    expect(state.visible).toEqual([]);
    await expect(drained).resolves.toBe(false);
    expect(state.stream.accept(event({ sequence: 2, token: "late" }))).toBe(false);
  });

  it("disposes listeners that arrive after teardown", () => {
    const state = controller();
    let unlistenCalls = 0;
    state.stream.teardown();
    state.stream.bindUnlisten(() => { unlistenCalls += 1; });
    expect(unlistenCalls).toBe(1);
  });
});

describe("chat response identity", () => {
  it("matches the exact immutable turn identity", () => {
    const identity = {
      sessionId: "session-1",
      turnId: "turn-1",
      generationToken: "generation-1",
    };
    expect(chatStreamResponseMatches(identity, {
      session_id: "session-1",
      turn_id: "turn-1",
      generation_token: "generation-1",
    })).toBe(true);
    expect(chatStreamResponseMatches(identity, {
      session_id: "session-1",
      turn_id: "turn-1",
      generation_token: "other",
    })).toBe(false);
  });
});
