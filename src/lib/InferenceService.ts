"use client";

import { invoke } from "@/lib/invoke";

interface InferenceProgress {
  status: string;
  progress: number; // 0 to 100
}

interface InferenceResult {
  text: string;
  latencyMs: number;
  device?: string;
}

type StreamMode = "unknown" | "raw" | "protocol";

const protocolRecordPattern = /^([A-Za-z0-9]):([\s\S]*)$/;
const sseHeaderPattern = /^(?:data|event|id|retry|content-type):/i;
const controlMarkupTokens = [
  "<|channel>thought",
  "<|channel>analysis",
  "<|channel>reasoning",
  "<|channel>text",
  "<|channel>final",
  "<|channel>answer",
  "<|channel>assistant",
  "<|channel>",
  "<channel|>",
  "</channel>",
  "<|turn>",
  "<turn|>",
  "</turn>",
  "<|think|>",
  "<|think>",
  "<|/think|>",
  "<|/think>",
  "<|message|>",
  "<|message>",
  "<|assistant|>",
  "<|assistant>",
  "<|model|>",
  "<|model>",
  "<|user|>",
  "<|user>",
  "<|system|>",
  "<|system>",
  "<|im_start|>",
  "<|im_start>",
  "<|im_end|>",
  "<|im_end>",
  "<|start_header_id|>",
  "<|end_header_id|>",
  "<|eot_id|>",
  "<|eot_id>",
  "<|begin_of_text|>",
  "<|begin_of_text>",
  "<|startoftext|>",
  "<|startoftext>",
  "<|endoftext|>",
  "<|endoftext>",
  "<|end_of_text|>",
  "<|end_of_text>",
  "<|end_of_turn|>",
  "<|end_of_turn>",
  "<text>",
  "</text>",
  "<start_of_turn>",
  "<end_of_turn>",
  "<start_of_image>",
  "<end_of_image>",
  "<image_soft_token>",
  "<|tool_call>",
  "<|tool_response>",
  "<|tool>",
  "<tool_call|>",
  "<tool_response|>",
  "<tool|>",
  "<|image|>",
  "<|image>",
  "<|audio|>",
  "<|audio>",
  "<|video|>",
  "<|video>",
  "<|\"|>",
  "<bos>",
  "<eos>",
  "<pad>",
  "[INST]",
  "[/INST]",
].sort((left, right) => right.length - left.length);
const maxControlMarkupTokenLength = Math.max(...controlMarkupTokens.map((token) => token.length));

function isAlphanumericCharacter(value: string) {
  return /^[\p{L}\p{N}]$/u.test(value);
}

function isAlphabeticCharacter(value: string) {
  return /^\p{L}$/u.test(value);
}

function isSentenceTerminalCharacter(value: string) {
  return /^[.,!?;:)\]\}"']$/.test(value);
}

const boundaryStandaloneWords = new Set([
  "a",
  "about",
  "after",
  "almost",
  "also",
  "am",
  "an",
  "and",
  "are",
  "as",
  "at",
  "be",
  "because",
  "been",
  "before",
  "between",
  "but",
  "by",
  "can",
  "could",
  "did",
  "do",
  "does",
  "for",
  "from",
  "go",
  "had",
  "has",
  "have",
  "he",
  "her",
  "here",
  "his",
  "how",
  "i",
  "if",
  "in",
  "into",
  "is",
  "it",
  "its",
  "leading",
  "me",
  "might",
  "my",
  "not",
  "now",
  "of",
  "on",
  "or",
  "our",
  "she",
  "should",
  "spacing",
  "still",
  "that",
  "the",
  "their",
  "then",
  "there",
  "these",
  "they",
  "this",
  "those",
  "to",
  "tokenization",
  "under",
  "was",
  "we",
  "were",
  "what",
  "when",
  "where",
  "which",
  "while",
  "who",
  "will",
  "with",
  "world",
  "would",
  "you",
  "your",
]);

const continuationFragments = new Set([
  "al",
  "ally",
  "ation",
  "ations",
  "ary",
  "ed",
  "er",
  "ers",
  "es",
  "est",
  "figuration",
  "ful",
  "ible",
  "ing",
  "ingly",
  "ization",
  "izations",
  "ize",
  "ized",
  "izes",
  "izing",
  "less",
  "ly",
  "ment",
  "ments",
  "ness",
  "ory",
  "ous",
  "ously",
  "pletely",
  "s",
  "sion",
  "sions",
  "tion",
  "tions",
  "uration",
  "ure",
  "ures",
]);

function endsWithUnclosedProtocolMarker(value: string) {
  const markerStart = value.lastIndexOf("<");
  return markerStart >= 0 && !value.slice(markerStart).includes(">");
}

function trailingAlphabeticWord(value: string) {
  const match = value.match(/[\p{L}]+$/u);
  return match?.[0] ?? "";
}

function leadingAlphabeticWord(value: string) {
  const match = value.match(/^[\p{L}]+/u);
  return match?.[0] ?? "";
}

function isLikelyContinuationFragment(value: string) {
  const normalized = value.toLowerCase();
  return continuationFragments.has(normalized);
}

function looksLikeMissingWordBoundary(previous: string, next: string) {
  const previousWord = trailingAlphabeticWord(previous).toLowerCase();
  const nextWord = leadingAlphabeticWord(next).toLowerCase();
  if (!previousWord || !nextWord) {
    return false;
  }
  if (isLikelyContinuationFragment(nextWord)) {
    return false;
  }
  if (boundaryStandaloneWords.has(previousWord) || boundaryStandaloneWords.has(nextWord)) {
    return true;
  }
  return previousWord.length >= 4 && nextWord.length >= 4;
}

function needsStreamChunkBoundarySpace(previous: string, next: string) {
  const previousCharacters = Array.from(previous);
  const nextCharacters = Array.from(next);
  const previousChar = previousCharacters[previousCharacters.length - 1];
  const nextChar = nextCharacters[0];
  if (!previousChar || !nextChar) {
    return false;
  }
  if (/\s/u.test(previousChar) || /\s/u.test(nextChar)) {
    return false;
  }
  if (endsWithUnclosedProtocolMarker(previous)) {
    return false;
  }
  if (isSentenceTerminalCharacter(previousChar) && isAlphanumericCharacter(nextChar)) {
    return true;
  }
  if (isAlphabeticCharacter(previousChar) && isAlphabeticCharacter(nextChar)) {
    return looksLikeMissingWordBoundary(previous, next);
  }
  return false;
}

function looksLikeProtocolStream(value: string) {
  const candidate = value.trimStart();
  return (
    /^(?:data:\s*)?[A-Za-z0-9]:/.test(candidate) ||
    /^(?:event|id|retry|content-type):/i.test(candidate) ||
    candidate.startsWith(":")
  );
}

function couldBeProtocolPrefix(value: string) {
  const candidate = value.trimStart().toLowerCase();
  if (/^[a-z0-9]$/.test(candidate) || /^[a-z0-9]:$/.test(candidate)) {
    return true;
  }
  return "data:".startsWith(candidate) || "event:".startsWith(candidate);
}

function unwrapSerializedProtocolChunk(value: string) {
  const candidate = value.trim();
  if (!candidate || (!candidate.startsWith("\"") && !candidate.startsWith("["))) {
    return value;
  }

  try {
    const parsed: unknown = JSON.parse(candidate);
    if (typeof parsed === "string" && (looksLikeProtocolStream(parsed) || couldBeProtocolPrefix(parsed))) {
      return parsed;
    }
    if (
      Array.isArray(parsed) &&
      parsed.every((entry) => typeof entry === "string") &&
      parsed.some((entry) => looksLikeProtocolStream(entry))
    ) {
      return parsed.join("\n");
    }
  } catch {
    // Partial JSON serialization is retained until a later chunk completes it.
  }

  return value;
}

function parseProtocolRecord(record: string, final: boolean) {
  let candidate = record.replace(/^\uFEFF/, "").trimStart();
  const trimmedCandidate = candidate.trim();
  if (!trimmedCandidate) {
    return { complete: true, recognized: true, text: "" };
  }
  if (candidate.startsWith(":")) {
    return { complete: true, recognized: true, text: "" };
  }
  if (/^(?:event|id|retry|content-type):/i.test(candidate)) {
    return { complete: true, recognized: true, text: "" };
  }
  if (/^data:/i.test(candidate)) {
    candidate = candidate.replace(/^data:[ \t]?/i, "");
  }
  if (!candidate.trim() || candidate.trim() === "[DONE]") {
    return { complete: true, recognized: true, text: "" };
  }

  const match = candidate.match(protocolRecordPattern);
  if (!match) {
    return { complete: final, recognized: sseHeaderPattern.test(candidate), text: "" };
  }

  const [, prefix, payload] = match;
  try {
    const decoded: unknown = JSON.parse(payload);
    return {
      complete: true,
      recognized: true,
      text: prefix === "0" && typeof decoded === "string" ? decoded : "",
    };
  } catch {
    return { complete: final, recognized: true, text: "" };
  }
}

function mergeInferenceTextChunk(previous: string, chunk: string) {
  if (!chunk) {
    return "";
  }
  const prefix = needsStreamChunkBoundarySpace(previous, chunk) ? " " : "";
  return `${prefix}${chunk}`;
}

function sanitizeControlMarkupText(value: string) {
  let sanitized = value;
  let removedControlMarkup = false;
  for (const token of controlMarkupTokens) {
    const replaced = replaceControlMarkupToken(sanitized, token);
    sanitized = replaced.text;
    removedControlMarkup ||= replaced.removed;
  }
  sanitized = sanitized.replace(/[ \t]{2,}/g, " ");
  if (sanitized.trim().length > 0) {
    return sanitized;
  }
  return removedControlMarkup ? "" : sanitized;
}

function replaceControlMarkupToken(value: string, token: string) {
  let remaining = value;
  let text = "";
  let removed = false;

  while (true) {
    const index = remaining.indexOf(token);
    if (index < 0) {
      text += remaining;
      return { text, removed };
    }

    text += remaining.slice(0, index);
    const after = remaining.slice(index + token.length);
    if (needsControlMarkupBoundary(text, after)) {
      text += " ";
    }
    remaining = after;
    removed = true;
  }
}

function needsControlMarkupBoundary(previousText: string, nextText: string) {
  const previous = Array.from(previousText).at(-1);
  const next = Array.from(nextText)[0];
  if (!previous || !next) {
    return false;
  }
  if (/\s/u.test(previous) || /\s/u.test(next)) {
    return false;
  }
  if (isSentenceTerminalCharacter(next)) {
    return false;
  }
  return true;
}

function trailingControlMarkupPrefixLength(value: string) {
  const maxLength = Math.min(value.length, maxControlMarkupTokenLength - 1);
  for (let length = maxLength; length > 0; length -= 1) {
    const suffix = value.slice(-length);
    if (controlMarkupTokens.some((token) => token.startsWith(suffix))) {
      return length;
    }
  }
  return 0;
}

class ControlMarkupStreamSanitizer {
  private pending = "";

  push(chunk: string) {
    if (!chunk) return "";

    const combined = `${this.pending}${chunk}`;
    const pendingLength = trailingControlMarkupPrefixLength(combined);
    const stable = pendingLength > 0 ? combined.slice(0, -pendingLength) : combined;
    this.pending = pendingLength > 0 ? combined.slice(-pendingLength) : "";
    return sanitizeControlMarkupText(stable);
  }

  finish() {
    const text = sanitizeControlMarkupText(this.pending);
    this.pending = "";
    return text;
  }
}

export class InferenceTextAccumulator {
  private text = "";
  private controlMarkup = new ControlMarkupStreamSanitizer();

  push(chunk: string) {
    const sanitizedChunk = this.controlMarkup.push(chunk);
    const text = mergeInferenceTextChunk(this.text, sanitizedChunk);
    this.text += text;
    return text;
  }

  value() {
    return this.text;
  }
}

export class InferenceStreamProcessor {
  private buffer = "";
  private protocolText = "";
  private mode: StreamMode = "unknown";
  private controlMarkup = new ControlMarkupStreamSanitizer();

  push(chunk: string): string {
    if (!chunk) return "";

    const normalizedChunk = unwrapSerializedProtocolChunk(chunk).replace(/^\uFEFF/, "");
    if (this.mode === "raw") {
      return looksLikeProtocolStream(normalizedChunk)
        ? sanitizeInferenceText(normalizedChunk)
        : this.controlMarkup.push(normalizedChunk);
    }

    this.buffer += normalizedChunk;
    if (this.mode === "unknown") {
      if (looksLikeProtocolStream(this.buffer)) {
        this.mode = "protocol";
      } else if (couldBeProtocolPrefix(this.buffer)) {
        return "";
      } else {
        this.mode = "raw";
        const text = this.buffer;
        this.buffer = "";
        return this.controlMarkup.push(text);
      }
    }

    return this.consumeProtocolBuffer(false);
  }

  finish(): string {
    if (!this.buffer) return this.controlMarkup.finish();
    if (this.mode === "unknown") {
      const normalized = unwrapSerializedProtocolChunk(this.buffer);
      if (!looksLikeProtocolStream(normalized)) {
        this.buffer = "";
        return this.controlMarkup.push(normalized) + this.controlMarkup.finish();
      }
      this.buffer = normalized;
      this.mode = "protocol";
    }
    if (this.mode === "raw") {
      const text = this.buffer;
      this.buffer = "";
      return this.controlMarkup.push(text) + this.controlMarkup.finish();
    }
    return this.consumeProtocolBuffer(true) + this.controlMarkup.finish();
  }

  private consumeProtocolBuffer(final: boolean): string {
    let text = "";

    while (this.buffer) {
      this.buffer = this.buffer.replace(/^\r?\n/, "");
      if (!this.buffer) break;

      const lineEnd = this.buffer.search(/\r?\n/);
      if (lineEnd >= 0) {
        const record = this.buffer.slice(0, lineEnd);
        const newlineLength = this.buffer.slice(lineEnd).startsWith("\r\n") ? 2 : 1;
        this.buffer = this.buffer.slice(lineEnd + newlineLength);
        text += this.mergeProtocolText(parseProtocolRecord(record, true).text);
        continue;
      }

      const parsed = parseProtocolRecord(this.buffer, final);
      if (!parsed.complete) break;
      text += this.mergeProtocolText(parsed.text);
      this.buffer = "";
    }

    return text;
  }

  private mergeProtocolText(chunk: string) {
    const text = mergeInferenceTextChunk(this.protocolText, chunk);
    this.protocolText += text;
    return this.controlMarkup.push(text);
  }
}

export function sanitizeInferenceText(value: string): string {
  const processor = new InferenceStreamProcessor();
  return processor.push(value) + processor.finish();
}

class InferenceService {
  /**
   * Run local inference.
   * Leverages stream_native_inference via Tauri event emitters and zero-copy streams.
   */
  async infer(
    prompt: string,
    onToken?: (token: string) => void,
    onProgress?: (progress: InferenceProgress) => void,
    context?: { sessionId?: string; systemPrompt?: string; signal?: AbortSignal }
  ): Promise<InferenceResult> {
    const startTime = performance.now();

    if (onProgress) {
      onProgress({
        status: "Dispatching native inference request...",
        progress: 50,
      });
    }

    let text = "";
    let deviceLabel = "Local Hardware";
    let unlisten: (() => void) | undefined;
    let streamedText = "";
    const streamProcessor = new InferenceStreamProcessor();
    const streamId = crypto.randomUUID();
    let abortHandler: (() => void) | undefined;

    const emitSanitizedToken = (token: string) => {
      if (!token) return;
      streamedText += token;
      onToken?.(token);
    };

    try {
      // 1. Hook into zero-copy Tauri event listeners for real-time streaming tokens
      if (onToken) {
        try {
          const { listen } = await import("@tauri-apps/api/event");
          unlisten = await listen<{ streamId?: unknown; token?: unknown }>("token-stream", (event) => {
            const chunk = event.payload;
            if (
              chunk &&
              chunk.streamId === streamId &&
              typeof chunk.token === "string"
            ) {
              emitSanitizedToken(streamProcessor.push(chunk.token));
            }
          });
        } catch (err) {
          console.warn("Unable to register token-stream event listener:", err);
        }
      }

      if (context?.signal) {
        if (context.signal.aborted) {
          throw new DOMException("Local generation was cancelled.", "AbortError");
        }
        abortHandler = () => {
          void invoke<boolean>("cancel_native_inference", { streamId }).catch(() => false);
        };
        context.signal.addEventListener("abort", abortHandler, { once: true });
      }

      // 2. Invoke native streaming command
      interface InferResponse {
        text: string;
        device?: string;
      }
      const response = await invoke<InferResponse>("stream_native_inference", {
        prompt,
        sessionId: context?.sessionId,
        systemPrompt: context?.systemPrompt,
        streamId,
      });
      emitSanitizedToken(streamProcessor.finish());
      text = sanitizeInferenceText(response.text) || streamedText;
      if (response.device) {
        deviceLabel = response.device;
      }
    } catch (error) {
      console.error("Native streaming inference failed:", error);
      throw error;
    } finally {
      if (unlisten) {
        unlisten();
      }
      if (abortHandler && context?.signal) {
        context.signal.removeEventListener("abort", abortHandler);
      }
    }

    if (onProgress) {
      onProgress({
        status: "Native inference completed.",
        progress: 100,
      });
    }

    const endTime = performance.now();
    const latencyMs = Math.round(endTime - startTime);

    return {
      text,
      latencyMs,
      device: deviceLabel,
    };
  }
}

export const inferenceService = new InferenceService();
