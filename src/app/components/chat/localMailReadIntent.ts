import {
  explicitlyTargetsAppleMessagesApp,
  hasCompetingReadOnlyPrivateAppTarget,
  hasPrivateAppMutationIntent,
  isFocusedLocalAppShortcutRequest,
  isInformationalLocalSystemTopicQuestion,
  isNaturalMailboxStateQuestion,
  targetsLocalMail,
} from "./localAppIntent";

type DirectLocalMailReadScope = "unread" | "recent" | "unread_or_today";

export type DirectLocalMailReadRequest = {
  unreadOnly: boolean;
  maxMessages: number;
  replyDraft: boolean;
  scope: DirectLocalMailReadScope;
};

export function detectDirectLocalMailReadRequest(
  message: string,
): DirectLocalMailReadRequest | null {
  const normalized = message.trim();
  if (!normalized || isInformationalLocalSystemTopicQuestion(normalized)) {
    return null;
  }
  if (
    explicitlyTargetsAppleMessagesApp(normalized) ||
    !isFocusedLocalAppShortcutRequest(normalized, "mail") ||
    hasCompetingReadOnlyPrivateAppTarget(normalized, "read_system_emails")
  ) {
    return null;
  }

  const lower = normalized.toLowerCase();
  const mentionsMail = /\b(mail|email|emails|e-mail|e-mails|inbox)\b/i.test(lower);
  const targetsPersonalMailbox = targetsLocalMail(lower);
  const asksToRead =
    /\b(check|find|look\s+for|look\s+up|lookup|read|review|scan|search|summari[sz]e|summary|report|show|list)\b/i.test(
      lower,
    ) || isNaturalMailboxStateQuestion(lower);
  const asksForReplyDraft =
    /\b(?:draft|write|prepare|compose)\b.{0,48}\brepl(?:y|ies)\b/i.test(lower) ||
    /\brepl(?:y|ies)\b.{0,48}\b(?:draft|write|prepare|compose)\b/i.test(lower);
  if (!mentionsMail || !targetsPersonalMailbox || (!asksToRead && !asksForReplyDraft)) {
    return null;
  }

  if (hasPrivateAppMutationIntent(lower)) {
    return null;
  }

  const asksForUnread = /\bunread\b/i.test(lower);
  const asksForToday =
    /\b(?:today|earlier\s+today|from\s+today|this\s+morning|this\s+afternoon|this\s+evening|tonight)\b/i.test(
      lower,
    );
  const unreadIndex = lower.search(/\bunread\b/i);
  const todayIndex = lower.search(
    /\b(?:earlier\s+today|from\s+(?:earlier\s+)?today|today|this\s+morning|this\s+afternoon|this\s+evening|tonight)\b/i,
  );
  const unreadOrToday =
    asksForUnread &&
    asksForToday &&
    unreadIndex >= 0 &&
    todayIndex >= 0 &&
    /\bor\b/i.test(
      lower.slice(Math.min(unreadIndex, todayIndex), Math.max(unreadIndex, todayIndex)),
    );
  const scope: DirectLocalMailReadScope =
    unreadOrToday
      ? "unread_or_today"
      : asksForUnread
        ? "unread"
        : "recent";

  return {
    unreadOnly: scope === "unread",
    maxMessages: scope === "unread_or_today" ? 50 : 20,
    replyDraft: asksForReplyDraft,
    scope,
  };
}
