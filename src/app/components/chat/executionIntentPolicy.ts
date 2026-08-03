import { sanitizeInferenceText } from "@/lib/InferenceService";

type NativeTaskIntentEvidence = {
  hasInternalMemoryIntent: boolean;
  hasLocalPath: boolean;
  mentionsAppleApp: boolean;
};

export type NativeEffectExpectation = {
  kind: "mutation" | "schedule";
  toolKey: string | null;
};

export type NativeEffectReceipt = {
  kind: "native_tool" | "workflow_schedule";
  effect: "read" | "mutation" | "schedule";
  toolKey: string | null;
  verified: boolean;
};

type NativeEffectRouteDecision = {
  decision_source: string;
  matched_signals: string[];
};

type VerifiedCommandResponse = {
  operation?: string;
  status: string;
  message: string;
  verified: boolean;
  claims?: string[];
};

type VerifiedCommandCopy = {
  failurePrefix: string;
  receiptPrefix: string;
  fileChangedBeforeSave?: string;
  filePreparationFailed?: string;
  fileVerificationFailed?: string;
};

function localizedVerifiedCommandFailure(message: string, copy: VerifiedCommandCopy) {
  const normalized = message.toLowerCase();
  if (
    normalized.includes("changed before oomu could save it") &&
    copy.fileChangedBeforeSave
  ) {
    return copy.fileChangedBeforeSave;
  }
  if (
    (normalized.includes("safe temporary file") ||
      normalized.includes("temporary file name is not valid") ||
      normalized.includes("approved external file write failed")) &&
    copy.filePreparationFailed
  ) {
    return copy.filePreparationFailed;
  }
  if (
    normalized.includes("verify the prepared file") &&
    copy.fileVerificationFailed
  ) {
    return copy.fileVerificationFailed;
  }
  return message;
}

const localNativeTaskSurfacePattern =
  /\b(?:mac|macos|computer|machine|local|finder|downloads?|documents|desktop|files?|folders?|directories|directory|path|filesystem|file system|workspace|repo|repository|terminal|shell|commands?|scripts?|apps?|applications?|mail|email|emails|inbox|calendar|agenda|schedule|events?|meetings?|appointments?|reminders?|tasks?|todos?|to-dos?|notes?|contacts?|address book|messages?|imessages?|safari|photos?|music|weather|maps|system settings|settings|clipboard|screen|screenshots?|screen recording|notifications?)\b/i;
const localNativeTaskActionPattern =
  /\b(?:add|activate|book|build|cancel|capture|check|compile|compose|copy|create|delete|diagnose|draft|edit|execute|find|install|launch|list|look\s+(?:at|for|up)|move|open|paste|read|record|remove|rename|review|run|save|scan|schedule|search|set|show|start|stop|summari[sz]e|take|test|troubleshoot|update|write)\b/i;
const localNativeTaskQuestionPattern =
  /\b(?:are\s+there|do\s+i\s+have|how\s+many|what(?:'s|\s+is)\s+(?:my|on|in|inside|scheduled|due)|when\s+(?:is|are)|where\s+(?:is|are))\b/i;
const retrospectiveNativeActionQuestionPattern =
  /(?:^|\b(?:explain|tell\s+me)\s+)\b(?:why|how|when|where|what)\s+(?:(?:did|do|does|would|could|was|were|is|are)\s+)?(?:you|oomu|the\s+app)\b|^(?:did|do|does|was|were|is|are)\s+(?:you|oomu|the\s+app)\b/i;
const firstPersonCompletedActuationPattern =
  /(?:^|[\n.!?]\s+)\s*(?:(?:done|okay|confirmed)[,;:\s—-]+)?(?:i|we)(?:(?:['’]ve)|\s+have)?\s+(?:(?:already|now|successfully)\s+)*(?:ran|run|executed|completed|saved|persisted|committed|wrote|written|created|added|scheduled|set\s+up|deleted|removed|installed|uninstalled|updated|modified|configured|connected|sent|opened|archived|moved|renamed|compiled|built|patched|fixed|uploaded|downloaded|exported|imported|recorded)\b/i;

export function nativeEffectExpectationForRouteDecision(
  decision: NativeEffectRouteDecision,
): NativeEffectExpectation | null {
  if (
    decision.decision_source === "routine_scheduler_filter" ||
    decision.matched_signals.some((signal) =>
      signal === "recurring routine" || signal === "future one-time routine")
  ) {
    return { kind: "schedule", toolKey: null };
  }
  if (
    decision.decision_source === "external_apple_app_write_filter" ||
    decision.matched_signals.includes("explicit Apple app write request")
  ) {
    return { kind: "mutation", toolKey: null };
  }
  return null;
}

export function bindNativeEffectExpectationToTool(
  expectation: NativeEffectExpectation | null,
  toolKey: string,
  mutating: boolean,
): NativeEffectExpectation | null {
  if (!mutating) {
    return expectation;
  }
  if (!expectation) {
    return { kind: "mutation", toolKey: toolKey.trim().toLowerCase() };
  }
  if (expectation.kind !== "mutation" || expectation.toolKey) return expectation;
  return { ...expectation, toolKey: toolKey.trim().toLowerCase() };
}

export function outstandingNativeEffectAfterReceipt(
  expectation: NativeEffectExpectation | null,
  receipt: NativeEffectReceipt,
): NativeEffectExpectation | null {
  if (!expectation || !receipt.verified) return expectation;
  if (expectation.kind === "schedule") {
    return receipt.kind === "workflow_schedule" && receipt.effect === "schedule"
      ? null
      : expectation;
  }
  const expectedTool = expectation.toolKey?.trim().toLowerCase() || null;
  const receivedTool = receipt.toolKey?.trim().toLowerCase() || null;
  return receipt.kind === "native_tool" &&
    receipt.effect === "mutation" &&
    expectedTool !== null &&
    expectedTool === receivedTool
    ? null
    : expectation;
}

export function shouldBlockOutstandingNativeEffectClaim(
  assistantText: string,
  userMessage: string,
  actionExpected: boolean,
  expectation: NativeEffectExpectation | null,
  verifiedReceipt: boolean,
) {
  const expected = actionExpected || expectation !== null;
  const receiptMatches = expectation === null && verifiedReceipt;
  return requiresPendingNativePostcondition(expected, receiptMatches) ||
    shouldBlockUnverifiedActionClaim(
      assistantText, userMessage, expected, receiptMatches,
    );
}
export function hasMutatingLocalIntent(message: string) {
  return /\b(write|edit|change|modify|delete|remove|create|compile|patch|fix)\b/i.test(message);
}

/**
 * A repeating instruction is workflow work, even when its first action looks
 * like a one-shot app read. This is a conservative shortcut guard: the planner
 * still owns the actual schedule interpretation and execution.
 */
export function hasRecurringAutomationIntent(message: string) {
  const normalized = message.trim();
  if (!normalized) return false;

  const hasCadence =
    /\b(?:hourly|daily|nightly|weekly|monthly|quarterly|yearly|annually|periodically|recurring|recurrent|repeatedly)\b/i.test(
      normalized,
    ) ||
    /\b(?:every|each)\s+(?:(?:other|\d+)\s+)?(?:minutes?|hours?|days?|nights?|weeks?|months?|quarters?|years?|weekdays?|weekends?|mornings?|afternoons?|evenings?|mondays?|tuesdays?|wednesdays?|thursdays?|fridays?|saturdays?|sundays?)\b/i.test(
      normalized,
    );
  const hasExecutableWork =
    /\b(?:check|read|review|scan|search|summari[sz]e|report|run|execute|send|create|update)\b/i.test(
      normalized,
    );
  return hasCadence && hasExecutableWork;
}

export function hasStructuralExecutionIntent(message: string) {
  return (
    hasRecurringAutomationIntent(message) ||
    /\b(run|execute)\b.{0,48}\b(command|script|binary|program|terminal|shell|workflow|taskflow|test|tests|build|compile|npm|npx|pnpm|yarn|cargo|python|node|bash|zsh|make)\b/i.test(message) ||
    /\b(command|script|binary|program|terminal|shell|workflow|taskflow|test|tests|build|compile|npm|npx|pnpm|yarn|cargo|python|node|bash|zsh|make)\b.{0,48}\b(run|execute)\b/i.test(message)
  );
}

export function isRetrospectiveNativeActionQuestion(message: string) {
  return retrospectiveNativeActionQuestionPattern.test(message.trim());
}

export function evaluateLikelyLocalNativeTaskIntent(
  message: string,
  evidence: NativeTaskIntentEvidence,
) {
  const normalized = message.trim();
  if (
    !normalized ||
    evidence.hasInternalMemoryIntent ||
    isRetrospectiveNativeActionQuestion(normalized)
  ) {
    return false;
  }
  if (hasStructuralExecutionIntent(normalized)) {
    return true;
  }
  const mentionsLocalSurface =
    evidence.hasLocalPath ||
    localNativeTaskSurfacePattern.test(normalized) ||
    evidence.mentionsAppleApp;
  return Boolean(
    mentionsLocalSurface &&
    (localNativeTaskActionPattern.test(normalized) ||
      /\buse\s+markdown\b/i.test(normalized) ||
      localNativeTaskQuestionPattern.test(normalized)),
  );
}

export function containsUnverifiedActionClaim(content: string) {
  return firstPersonCompletedActuationPattern.test(sanitizeInferenceText(content));
}

export function requiresPendingNativePostcondition(
  executableActionExpected: boolean,
  verifiedExecutionReceiptAvailable: boolean,
) {
  return executableActionExpected && !verifiedExecutionReceiptAvailable;
}

export function shouldBlockUnverifiedActionClaim(
  assistantContent: string,
  userPrompt: string,
  executableActionExpected: boolean,
  verifiedExecutionReceiptAvailable: boolean,
) {
  return Boolean(
    containsUnverifiedActionClaim(assistantContent) &&
    !isRetrospectiveNativeActionQuestion(userPrompt) &&
    executableActionExpected &&
    !verifiedExecutionReceiptAvailable,
  );
}

export function directExecuteCommandText(
  response: VerifiedCommandResponse,
  fallbackFailure: string,
  copy: VerifiedCommandCopy,
) {
  const message = response.message.trim();
  const claims = (response.claims ?? []).map((claim) => claim.trim()).filter(Boolean);
  if (response.status === "completed" && response.verified) {
    if (message) {
      return message;
    }
    if (claims.length > 0) {
      return `${copy.receiptPrefix}:\n${claims.join("\n")}`;
    }
  }
  const failure = message
    ? localizedVerifiedCommandFailure(message, copy)
    : fallbackFailure;
  return `${copy.failurePrefix} ${failure}`;
}
