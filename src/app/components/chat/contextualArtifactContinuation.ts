type TranscriptEntry = {
  role: string;
  content: string;
};

type ContextualArtifactRouteDecision = {
  route: "agentic_planner" | "conversational_stream";
  requires_local_access: boolean;
  decision_source: string;
  reason: string;
  matched_signals: string[];
  status_label: string;
};

const MARKDOWN_CREATION = /\b(?:write|create|save|put|export|make|generate|produce)\b/i;

function isAgentOwnedMarkdownRequest(message: string) {
  const normalized = message.toLowerCase();
  return (normalized.includes("markdown") || normalized.includes(".md")) &&
    MARKDOWN_CREATION.test(message);
}

export function isAgentOwnedMarkdownDestinationDeferred(message: string) {
  return isAgentOwnedMarkdownRequest(message) &&
    /\bi\s+(?:will|'ll)\s+(?:give|provide|send|share)\s+(?:you\s+)?(?:the\s+)?(?:destination|path|folder)\s+(?:next|later)\b/i.test(message);
}

function isAbsoluteDirectoryOnlyTurn(message: string) {
  const trimmed = message.trim().replace(/^([`'"])([\s\S]*)\1$/, "$2").trim();
  return trimmed.startsWith("/") && !trimmed.includes("\n") && trimmed.length > 1;
}

export function isContextualMarkdownDestinationContinuation(
  message: string,
  transcript: readonly TranscriptEntry[],
) {
  if (!isAbsoluteDirectoryOnlyTurn(message)) return false;
  const priorUserTurn = [...transcript]
    .reverse()
    .find((entry) => entry.role === "user" && entry.content.trim());
  return Boolean(priorUserTurn && isAgentOwnedMarkdownRequest(priorUserTurn.content));
}

export function contextualArtifactRouteDecision(
  message: string,
  transcript: readonly TranscriptEntry[],
  statusLabel: string,
): ContextualArtifactRouteDecision | null {
  if (isAgentOwnedMarkdownDestinationDeferred(message)) {
    return {
      route: "conversational_stream",
      requires_local_access: false,
      decision_source: "deferred_contextual_markdown_destination",
      reason: "The user explicitly said the destination will arrive in the next turn.",
      matched_signals: ["markdown_request", "destination_deferred"],
      status_label: statusLabel,
    };
  }
  if (!isContextualMarkdownDestinationContinuation(message, transcript)) return null;
  return {
    route: "agentic_planner",
    requires_local_access: true,
    decision_source: "persisted_contextual_markdown_destination",
    reason: "The current directory completes the immediately preceding Markdown creation request.",
    matched_signals: ["persisted_markdown_request", "absolute_directory_followup"],
    status_label: statusLabel,
  };
}

export function contextualArtifactTurnRouting(
  message: string,
  transcript: readonly TranscriptEntry[],
  statusLabel: string,
  enabled: boolean,
  baselineNativeIntent: boolean,
  baselinePlannerFallback: boolean,
) {
  const route = enabled
    ? contextualArtifactRouteDecision(message, transcript, statusLabel)
    : null;
  return {
    route,
    likelyLocalNativeTaskIntent: route
      ? route.route === "agentic_planner"
      : baselineNativeIntent,
    plannerConversationFallbackAllowed: route ? false : baselinePlannerFallback,
  };
}

export function preferContextualArtifactRoute<T>(contextual: T | null, fallback: T) {
  return contextual ?? fallback;
}

export function plannerConversationFallbackAllowed(
  recoveryPlan: boolean,
  hasDirectLocalCommand: boolean,
  hasPrivateAppCandidate: boolean,
  hasLikelyNativeIntent: boolean,
  hasBrowserNavigationIntent: boolean,
) {
  return !recoveryPlan && !hasDirectLocalCommand && !hasPrivateAppCandidate &&
    !hasLikelyNativeIntent && !hasBrowserNavigationIntent;
}
