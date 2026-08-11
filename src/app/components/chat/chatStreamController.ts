import { InferenceTextAccumulator } from "@/lib/InferenceService";
import {
  BrowserControlEnvelopeAccumulator,
  type BrowserControlEnvelopeUpdate,
} from "./browserControlEnvelope";

export type ChatStreamIdentity = {
  streamId: string;
  sessionId: string;
  turnId: string;
  generationToken: string;
};

type NativeChatStreamIdentity = {
  stream_id: string;
  session_id: string;
  turn_id: string;
  generation_token: string;
};

export type ChatTokenEvent = NativeChatStreamIdentity & {
  sequence: number;
  token: string;
  elapsed_ms: number;
  delivery_state: "provisional" | "validated";
};

export type ChatValidatedStreamCompleteEvent = NativeChatStreamIdentity & {
  last_sequence: number;
  chunk_count: number;
  text_sha256: string;
  delivery_state: "validated";
};

type FrameScheduler = {
  request(callback: FrameRequestCallback): number;
  cancel(frame: number): void;
};

export type ChatStreamControllerOptions = {
  identity: ChatStreamIdentity;
  ownsTurn: () => boolean;
  requiresNativeReceipt: boolean;
  onVisibleText: (chunk: string) => void;
  onDirectiveSnapshot: (snapshot: string) => boolean;
  onFirstToken: () => void;
  scheduler?: FrameScheduler;
};

type StreamedAssistantMessage = {
  id: number;
  content: string;
  isPending?: boolean;
};

type ProjectedChatStreamOptions<
  Message extends StreamedAssistantMessage,
  DirectiveGrant,
  DirectiveRoute,
> = {
  streamId: string;
  turn: Pick<ChatStreamIdentity, "sessionId" | "turnId" | "generationToken">;
  ownsTurn: () => boolean;
  requiresNativeReceipt: boolean;
  assistantMessageId: number;
  updateMessages: (update: (messages: Message[]) => Message[]) => unknown;
  directiveSessionId: string;
  directiveGrants: readonly DirectiveGrant[];
  activateDirective: (
    snapshot: string,
    messageId: number,
    sessionId: string,
    grants: readonly DirectiveGrant[],
    activateRoute: DirectiveRoute,
  ) => boolean;
  activateRoute: DirectiveRoute;
  onFirstToken: () => void;
};

type ChatStreamResponseIdentity = {
  session_id: string;
  turn_id: string;
  generation_token: string;
};

type DrainWaiter = {
  expectedText: string;
  expectedDigest: string | null;
  deadlineAt: number;
  timer: ReturnType<typeof setTimeout>;
  resolve: (accepted: boolean) => void;
};

const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const TERMINAL_ARRIVAL_GRACE_MS = 250;
const MIN_VISIBLE_CHUNK_INTERVAL_MS = 20;
const TARGET_VISIBLE_BURST_DURATION_MS = 3_000;
const MAX_VISIBLE_CHUNK_INTERVAL_MS = 250;
const DEFAULT_DRAIN_TIMEOUT_MS = 5_000;
const MAX_DRAIN_TIMEOUT_MS = 15_000;

export function chatStreamResponseMatches(
  identity: Pick<ChatStreamIdentity, "sessionId" | "turnId" | "generationToken">,
  response: ChatStreamResponseIdentity,
) {
  return (
    response.session_id === identity.sessionId &&
    response.turn_id === identity.turnId &&
    response.generation_token === identity.generationToken
  );
}

export async function rejectStaleResponse<Identity extends Pick<ChatStreamIdentity, "sessionId" | "turnId" | "generationToken">>(
  identity: Identity,
  response: ChatStreamResponseIdentity,
  turnIsCurrent: (identity: Identity) => boolean,
  steerSupersedesTurn: (identity: Identity) => boolean,
  preserveTerminal: (identity: Identity) => boolean,
  abandon: (identity: Identity, hydrationLockToken: number | null) => Promise<void>,
  hydrationLockToken: number | null,
) {
  const accepted =
    turnIsCurrent(identity) &&
    !steerSupersedesTurn(identity) &&
    chatStreamResponseMatches(identity, response);
  if (accepted) return false;
  if (!preserveTerminal(identity)) await abandon(identity, hydrationLockToken);
  return true;
}

function browserFrameScheduler(): FrameScheduler {
  return {
    request: (callback) => window.requestAnimationFrame(callback),
    cancel: (frame) => window.cancelAnimationFrame(frame),
  };
}

function eventMatchesIdentity(event: NativeChatStreamIdentity, identity: ChatStreamIdentity) {
  return (
    event.stream_id === identity.streamId &&
    event.session_id === identity.sessionId &&
    event.turn_id === identity.turnId &&
    event.generation_token === identity.generationToken
  );
}

async function textSha256(text: string) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("");
}

/**
 * The single production state machine for regular and steered chat token streams.
 * Only native-validated chunks can reach projection. Terminal authority reconciles
 * the exact text; the final native response remains authoritative to the caller.
 */
export class ChatStreamController {
  private readonly identity: ChatStreamIdentity;
  private readonly ownsTurn: () => boolean;
  private readonly requiresNativeReceipt: boolean;
  private readonly onVisibleText: (chunk: string) => void;
  private readonly onDirectiveSnapshot: (snapshot: string) => boolean;
  private readonly onFirstToken: () => void;
  private readonly scheduler: FrameScheduler;
  private readonly text = new InferenceTextAccumulator();
  private readonly controlEnvelope = new BrowserControlEnvelopeAccumulator();
  private readonly validatedQueue: string[] = [];
  private readonly waiters = new Set<DrainWaiter>();
  private readonly unlisteners: Array<() => void> = [];
  private frame: number | null = null;
  private lastProvisionalSequence = 0;
  private lastValidatedSequence = 0;
  private validatedTokenCount = 0;
  private drainedTokenCount = 0;
  private acceptedValidatedText = "";
  private drainedValidatedText = "";
  private terminal: ChatValidatedStreamCompleteEvent | null = null;
  private pendingTerminal: ChatValidatedStreamCompleteEvent | null = null;
  private firstProjectionAt: number | null = null;
  private lastProjectionAt: number | null = null;
  private visibleChunkIntervalMs = MIN_VISIBLE_CHUNK_INTERVAL_MS;
  private firstTokenSeen = false;
  private directiveActivated = false;
  private active = true;
  private listenersClosed = false;
  private listenerSetupFailed = false;

  constructor(options: ChatStreamControllerOptions) {
    this.identity = options.identity;
    this.ownsTurn = options.ownsTurn;
    this.requiresNativeReceipt = options.requiresNativeReceipt;
    this.onVisibleText = options.onVisibleText;
    this.onDirectiveSnapshot = options.onDirectiveSnapshot;
    this.onFirstToken = options.onFirstToken;
    this.scheduler = options.scheduler ?? browserFrameScheduler();
  }

  bindUnlisten(unlisten: () => void) {
    if (!this.active || this.listenersClosed) {
      unlisten();
      return;
    }
    this.unlisteners.push(unlisten);
  }

  async listen() {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      this.bindUnlisten(await listen<ChatTokenEvent>("chat://token", (event) => {
        this.accept(event.payload);
      }));
      this.bindUnlisten(await listen<ChatValidatedStreamCompleteEvent>(
        "chat://validated-stream-complete",
        (event) => {
          this.acceptTerminal(event.payload);
        },
      ));
      return this.active;
    } catch {
      this.listenerSetupFailed = true;
      this.closeListeners();
      // The caller reconciles from the genuine native final response.
      return false;
    }
  }

  accept(event: ChatTokenEvent) {
    if (
      !this.active ||
      !this.ownsTurn() ||
      this.terminal ||
      !eventMatchesIdentity(event, this.identity) ||
      !event.token ||
      !Number.isSafeInteger(event.sequence) ||
      event.sequence <= 0
    ) {
      return false;
    }

    if (event.delivery_state === "provisional") {
      if (event.sequence <= this.lastProvisionalSequence) return false;
      this.lastProvisionalSequence = event.sequence;
      this.noteProgress();
      return true;
    }
    if (
      event.delivery_state !== "validated" ||
      event.sequence !== this.lastValidatedSequence + 1
    ) {
      return false;
    }

    this.lastValidatedSequence = event.sequence;
    this.validatedTokenCount += 1;
    this.acceptedValidatedText += event.token;
    this.validatedQueue.push(event.token);
    this.noteProgress();
    this.scheduleFrame();
    this.promotePendingTerminal();
    return true;
  }

  acceptTerminal(event: ChatValidatedStreamCompleteEvent) {
    if (
      !this.active ||
      !this.ownsTurn() ||
      this.terminal ||
      this.pendingTerminal ||
      !eventMatchesIdentity(event, this.identity) ||
      event.delivery_state !== "validated" ||
      !Number.isSafeInteger(event.last_sequence) ||
      !Number.isSafeInteger(event.chunk_count) ||
      event.chunk_count <= 0 ||
      event.last_sequence !== event.chunk_count ||
      !SHA256_PATTERN.test(event.text_sha256) ||
      event.last_sequence < this.lastValidatedSequence
    ) {
      return false;
    }
    if (event.last_sequence > this.lastValidatedSequence) {
      this.pendingTerminal = { ...event };
      this.extendWaitersForDrain();
      return true;
    }
    return this.finalizeTerminal(event);
  }

  private finalizeTerminal(event: ChatValidatedStreamCompleteEvent) {
    if (
      event.last_sequence !== this.lastValidatedSequence ||
      event.chunk_count !== this.validatedTokenCount
    ) {
      return false;
    }
    this.adaptTerminalBurstCadence();
    this.terminal = { ...event };
    this.extendWaitersForDrain();
    this.settleWaiters();
    return true;
  }

  awaitValidatedDrain(
    expectedText: string,
    timeoutMs = DEFAULT_DRAIN_TIMEOUT_MS,
    expectsValidatedStream = true,
  ): Promise<boolean> {
    if (
      !expectsValidatedStream ||
      !this.active ||
      !this.ownsTurn() ||
      this.listenerSetupFailed
    ) {
      return Promise.resolve(false);
    }
    const boundedTimeout = Math.min(Math.max(1, timeoutMs), MAX_DRAIN_TIMEOUT_MS);
    const deadlineAt = performance.now() + boundedTimeout;
    return new Promise((resolve) => {
      const waiter: DrainWaiter = {
        expectedText,
        expectedDigest: null,
        deadlineAt,
        timer: setTimeout(
          () => this.resolveWaiter(waiter, false),
          this.terminal || this.pendingTerminal
            ? boundedTimeout
            : Math.min(TERMINAL_ARRIVAL_GRACE_MS, boundedTimeout),
        ),
        resolve,
      };
      this.waiters.add(waiter);
      void textSha256(expectedText).then((digest) => {
        if (!this.waiters.has(waiter)) return;
        waiter.expectedDigest = digest;
        this.settleWaiters();
      }).catch(() => this.resolveWaiter(waiter, false));
    });
  }

  teardown() {
    if (!this.active) return;
    this.closeListeners();
    if (this.frame !== null) {
      this.scheduler.cancel(this.frame);
      this.frame = null;
    }
    this.validatedQueue.length = 0;
    this.pendingTerminal = null;
    this.finishControlEnvelope();
    this.active = false;
    this.failWaiters();
  }

  private noteProgress() {
    if (this.firstTokenSeen) return;
    this.firstTokenSeen = true;
    this.onFirstToken();
  }

  private scheduleFrame() {
    if (this.frame !== null || this.validatedQueue.length === 0) return;
    this.frame = this.scheduler.request((timestamp) => this.drainOneFrame(timestamp));
  }

  private drainOneFrame(timestamp: number) {
    this.frame = null;
    if (!this.active || !this.ownsTurn()) {
      this.validatedQueue.length = 0;
      this.failWaiters();
      return;
    }
    if (
      this.lastProjectionAt !== null &&
      timestamp - this.lastProjectionAt < this.visibleChunkIntervalMs
    ) {
      this.scheduleFrame();
      return;
    }
    const token = this.validatedQueue.shift();
    if (token !== undefined) {
      this.firstProjectionAt ??= timestamp;
      this.lastProjectionAt = timestamp;
      this.drainedTokenCount += 1;
      this.drainedValidatedText += token;
      this.projectControlEnvelope(this.controlEnvelope.push(this.text.push(token)));
    }
    this.scheduleFrame();
    this.settleWaiters();
  }

  private adaptTerminalBurstCadence() {
    const queuedGaps = this.validatedQueue.length - (this.drainedTokenCount === 0 ? 1 : 0);
    const minimumBufferedChunkCount = Math.ceil(
      TARGET_VISIBLE_BURST_DURATION_MS / MAX_VISIBLE_CHUNK_INTERVAL_MS,
    );
    if (queuedGaps <= 0 || this.validatedTokenCount < minimumBufferedChunkCount) return;
    const visibleElapsed = this.firstProjectionAt === null || this.lastProjectionAt === null
      ? 0
      : this.lastProjectionAt - this.firstProjectionAt;
    const remainingDuration = Math.max(0, TARGET_VISIBLE_BURST_DURATION_MS - visibleElapsed);
    this.visibleChunkIntervalMs = Math.min(MAX_VISIBLE_CHUNK_INTERVAL_MS, Math.max(
      MIN_VISIBLE_CHUNK_INTERVAL_MS, remainingDuration / queuedGaps,
    ));
  }

  private finishControlEnvelope() {
    if (this.active && this.ownsTurn()) {
      this.projectControlEnvelope(this.controlEnvelope.finish());
    }
  }

  private projectControlEnvelope(update: BrowserControlEnvelopeUpdate) {
    if (
      update.directiveChanged &&
      !this.directiveActivated &&
      update.directiveSnapshot
    ) {
      this.directiveActivated = this.onDirectiveSnapshot(update.directiveSnapshot);
    }
    if (!this.requiresNativeReceipt && update.visibleDelta) {
      this.onVisibleText(update.visibleDelta);
    }
  }

  private promotePendingTerminal() {
    if (
      !this.pendingTerminal ||
      this.pendingTerminal.last_sequence !== this.lastValidatedSequence ||
      this.pendingTerminal.chunk_count !== this.validatedTokenCount
    ) {
      return;
    }
    const terminal = this.pendingTerminal;
    this.pendingTerminal = null;
    this.finalizeTerminal(terminal);
  }

  private settleWaiters() {
    if (!this.active || !this.ownsTurn()) {
      this.failWaiters();
      return;
    }
    if (!this.terminal) return;
    for (const waiter of [...this.waiters]) {
      if (!waiter.expectedDigest) continue;
      const exact =
        this.terminal.text_sha256 === waiter.expectedDigest &&
        this.acceptedValidatedText === waiter.expectedText;
      if (!exact) {
        this.resolveWaiter(waiter, false);
        continue;
      }
      if (
        this.frame === null &&
        this.validatedQueue.length === 0 &&
        this.drainedTokenCount === this.validatedTokenCount &&
        this.drainedValidatedText === waiter.expectedText
      ) {
        this.resolveWaiter(waiter, true);
      }
    }
  }

  private extendWaitersForDrain() {
    for (const waiter of this.waiters) {
      clearTimeout(waiter.timer);
      const remaining = waiter.deadlineAt - performance.now();
      if (remaining <= 0) {
        this.resolveWaiter(waiter, false);
        continue;
      }
      waiter.timer = setTimeout(
        () => this.resolveWaiter(waiter, false),
        remaining,
      );
    }
  }

  private resolveWaiter(waiter: DrainWaiter, accepted: boolean) {
    if (!this.waiters.delete(waiter)) return;
    clearTimeout(waiter.timer);
    waiter.resolve(accepted);
  }

  private failWaiters() {
    for (const waiter of [...this.waiters]) this.resolveWaiter(waiter, false);
  }

  private closeListeners() {
    if (this.listenersClosed) return;
    this.listenersClosed = true;
    for (const unlisten of this.unlisteners.splice(0)) {
      try {
        unlisten();
      } catch {
        // Every other listener must still be released.
      }
    }
  }
}

export function createProjectedChatStreamController<
  Message extends StreamedAssistantMessage,
  DirectiveGrant,
  DirectiveRoute,
>(options: ProjectedChatStreamOptions<Message, DirectiveGrant, DirectiveRoute>) {
  return new ChatStreamController({
    identity: { streamId: options.streamId, ...options.turn },
    ownsTurn: options.ownsTurn,
    requiresNativeReceipt: options.requiresNativeReceipt,
    onVisibleText: (chunk) => {
      options.updateMessages((messages) => messages.map((message) =>
        message.id === options.assistantMessageId
          ? { ...message, content: message.content + chunk, isPending: false }
          : message,
      ));
    },
    onDirectiveSnapshot: (snapshot) =>
      options.activateDirective(
        snapshot,
        options.assistantMessageId,
        options.directiveSessionId,
        options.directiveGrants,
        options.activateRoute,
      ),
    onFirstToken: options.onFirstToken,
  });
}
