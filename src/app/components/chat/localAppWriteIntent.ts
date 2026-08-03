import {
  explicitlyTargetsLocalNotesApp,
  isFocusedLocalAppShortcutRequest,
  targetsLocalReminders,
} from "./localAppIntent";

type DirectLocalAppleAppWriteRequest = {
  toolName: string;
  argumentsValue: Record<string, unknown>;
  appLabel: string;
};

export function nativeAppleAppApprovalPresentation(
  toolName: string,
  argumentsValue: unknown,
) {
  if (toolName !== "draft_system_email" || argumentsValue === undefined) return null;
  try {
    return {
      actionType: "draft_system_email",
      actionLabel: "Save a Mail draft",
      preview: JSON.stringify(argumentsValue),
    };
  } catch {
    return null;
  }
}

export function detectDirectLocalAppleAppWriteRequest(
  message: string,
): DirectLocalAppleAppWriteRequest | null {
  const normalized = message.trim();
  if (!normalized || isInternalAgentMemoryRequest(normalized)) {
    return null;
  }
  const lower = normalized.toLowerCase();

  if (
    explicitlyTargetsLocalNotesApp(lower) &&
    /\b(add|create|make|save|write)\b/i.test(lower)
  ) {
    if (!isFocusedLocalAppShortcutRequest(normalized, "notes")) return null;
    return {
      toolName: "create_system_note",
      argumentsValue: {
        title: "OOMU note",
        body: appleAppWriteBodyFromPrompt(normalized) || normalized,
      },
      appLabel: "Notes",
    };
  }

  if (targetsLocalReminders(lower) && /\b(add|create|make|remind|set)\b/i.test(lower)) {
    if (!isFocusedLocalAppShortcutRequest(normalized, "reminders")) return null;
    return {
      toolName: "add_system_reminder",
      argumentsValue: { title: reminderTitleFromPrompt(normalized) || normalized },
      appLabel: "Reminders",
    };
  }

  if (
    /\b(mail|email|e-mail)\b/i.test(lower) &&
    /\b(?:open|create|compose)\b.{0,40}\b(?:mail|email|e-mail)\b.{0,40}\bdraft\b/i.test(lower)
  ) {
    if (!isFocusedLocalAppShortcutRequest(normalized, "mail")) return null;
    const body = appleAppWriteBodyFromPrompt(normalized);
    if (!body) return null;
    return {
      toolName: "draft_system_email",
      argumentsValue: { subject: "Draft from OOMU", body },
      appLabel: "Mail",
    };
  }

  return null;
}

export function isInternalAgentMemoryRequest(message: string) {
  const normalized = message.trim().toLowerCase();
  if (!normalized) return false;

  const noteIdiom = /\b(?:make|take)\s+(?:a\s+)?note\s+of\b/i.test(normalized);
  const explicitlyTargetsAppleNotes =
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:add|create|make|save|write)\s+(?:an?\s+)?apple\s+note\b/i.test(normalized) ||
    /\b(?:in|into|to|inside|within)\s+(?:(?:my|the)\s+)?(?:apple\s+)?notes?(?:\s+(?:app|application))?\b/i.test(normalized);
  const explicitlyTargetsAgentMemory =
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:add|create|make|save|store|put|keep|record|write)\b[\s\S]{0,160}\b(?:your(?:\s+oomu(?:'s)?)?|agent(?:'s)?|oomu(?:'s)?)\s+(?:long[-\s]?term\s+)?memor(?:y|ies)\b/i.test(normalized);
  const requestsNotesWrite =
    /\bnotes?\b/i.test(normalized) &&
    /\b(?:add|create|make|save|write)\b/i.test(normalized) &&
    (!noteIdiom || explicitlyTargetsAppleNotes);
  const requestsReminderWrite =
    /\b(?:reminders?|tasks?)\b/i.test(normalized) &&
    /\b(?:add|create|make|remind|set)\b/i.test(normalized);
  const requestsMailWrite =
    /\b(?:mail|email|e-mail)\b/i.test(normalized) &&
    /\b(?:open|create|compose)\b[\s\S]{0,40}\b(?:mail|email|e-mail)\b[\s\S]{0,40}\bdraft\b/i.test(normalized);
  if (
    requestsReminderWrite ||
    requestsMailWrite ||
    (requestsNotesWrite && (!explicitlyTargetsAgentMemory || explicitlyTargetsAppleNotes))
  ) {
    return false;
  }

  const asksAgentToRemember =
    explicitlyTargetsAgentMemory ||
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?)|(?:i\s+(?:want|need|would\s+like)\s+you\s+to\s+))?(?:remember|memorize|memorise)\s+\S/i.test(normalized) ||
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:make|take)\s+(?:a\s+)?note\s+of\s+\S/i.test(normalized);
  const directlySetsUserIdentity =
    /^(?:yes[\s,!-]*)?(?:please\s+)?(?:you\s+can\s+|(?:can|could|would|will)\s+you\s+)?call\s+me\b/i.test(normalized) ||
    /\bfrom\s+now\s+on[\s,]+call\s+me\b/i.test(normalized) ||
    /^(?:please\s+)?(?:remember\s+that\s+)?my\s+name\s+is\b/i.test(normalized);

  return asksAgentToRemember || directlySetsUserIdentity;
}

function appleAppWriteBodyFromPrompt(message: string) {
  return /\b(?:saying|that says|with(?: the)?(?: body| text| content)?|:)\s+([\s\S]+)$/i
    .exec(message)?.[1]?.trim() || null;
}

function reminderTitleFromPrompt(message: string) {
  return /\b(?:remind me to|reminder to|task to|to-do to|todo to|add a reminder to|create a reminder to)\s+([\s\S]+)$/i
    .exec(message)?.[1]?.trim() || appleAppWriteBodyFromPrompt(message);
}
