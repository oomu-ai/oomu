export const ATTACHMENT_LIMITS = Object.freeze({
  maxCount: 5,
  maxDecodedBytes: 20 * 1024 * 1024,
  maxEncodedBytes: 28 * 1024 * 1024,
  maxFileBytes: 8 * 1024 * 1024,
  maxDimension: 8192,
  maxPixels: 40_000_000,
  concurrency: 2,
});

export type AttachmentCandidate<T> = {
  name: string;
  decodedByteCount: number;
  encodedByteCount: number;
  width?: number | null;
  height?: number | null;
  process: (signal: AbortSignal) => Promise<T>;
  release?: () => void;
};

type AttachmentProcessResult<T> =
  | { ok: true; name: string; value: T }
  | { ok: false; name: string; errorCode: string };

type AttachmentUsage = {
  count: number;
  decodedBytes: number;
  encodedBytes: number;
};

const emptyUsage: AttachmentUsage = {
  count: 0,
  decodedBytes: 0,
  encodedBytes: 0,
};

function preflightError<T>(
  candidate: AttachmentCandidate<T>,
  usage: AttachmentUsage,
) {
  if (usage.count + 1 > ATTACHMENT_LIMITS.maxCount) return "attachment_count_limit_exceeded";
  if (
    !Number.isSafeInteger(candidate.decodedByteCount) ||
    candidate.decodedByteCount < 0 ||
    candidate.decodedByteCount > ATTACHMENT_LIMITS.maxFileBytes
  ) {
    return "attachment_file_byte_limit_exceeded";
  }
  if (
    !Number.isSafeInteger(candidate.encodedByteCount) ||
    candidate.encodedByteCount < 0 ||
    usage.encodedBytes + candidate.encodedByteCount > ATTACHMENT_LIMITS.maxEncodedBytes
  ) {
    return "attachment_encoded_byte_limit_exceeded";
  }
  if (usage.decodedBytes + candidate.decodedByteCount > ATTACHMENT_LIMITS.maxDecodedBytes) {
    return "attachment_aggregate_byte_limit_exceeded";
  }
  const width = candidate.width ?? 0;
  const height = candidate.height ?? 0;
  if (
    width < 0 ||
    height < 0 ||
    width > ATTACHMENT_LIMITS.maxDimension ||
    height > ATTACHMENT_LIMITS.maxDimension ||
    width * height > ATTACHMENT_LIMITS.maxPixels
  ) {
    return "attachment_image_dimension_limit_exceeded";
  }
  return null;
}

export async function processAttachmentsBounded<T>(
  candidates: AttachmentCandidate<T>[],
  options: { signal: AbortSignal; usage?: AttachmentUsage },
): Promise<AttachmentProcessResult<T>[]> {
  const results = new Array<AttachmentProcessResult<T>>(candidates.length);
  const accepted: Array<{ candidate: AttachmentCandidate<T>; index: number }> = [];
  const usage = { ...(options.usage ?? emptyUsage) };

  // All aggregate limits are decided from metadata before any process callback
  // is entered, so a rejected batch cannot partially read an oversized file.
  candidates.forEach((candidate, index) => {
    const errorCode = preflightError(candidate, usage);
    if (errorCode) {
      candidate.release?.();
      results[index] = { ok: false, name: candidate.name, errorCode };
      return;
    }
    usage.count += 1;
    usage.decodedBytes += candidate.decodedByteCount;
    usage.encodedBytes += candidate.encodedByteCount;
    accepted.push({ candidate, index });
  });

  let cursor = 0;
  async function worker() {
    while (!options.signal.aborted) {
      const next = accepted[cursor++];
      if (!next) return;
      try {
        const value = await next.candidate.process(options.signal);
        if (options.signal.aborted) {
          next.candidate.release?.();
          results[next.index] = {
            ok: false,
            name: next.candidate.name,
            errorCode: "attachment_processing_cancelled",
          };
        } else {
          results[next.index] = { ok: true, name: next.candidate.name, value };
        }
      } catch {
        next.candidate.release?.();
        results[next.index] = {
          ok: false,
          name: next.candidate.name,
          errorCode: options.signal.aborted
            ? "attachment_processing_cancelled"
            : "attachment_processing_failed",
        };
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(ATTACHMENT_LIMITS.concurrency, accepted.length) },
      () => worker(),
    ),
  );
  for (const next of accepted) {
    if (!results[next.index]) {
      next.candidate.release?.();
      results[next.index] = {
        ok: false,
        name: next.candidate.name,
        errorCode: "attachment_processing_cancelled",
      };
    }
  }
  return results;
}
