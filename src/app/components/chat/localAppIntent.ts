import { invoke, isTauriRuntime } from "@/lib/invoke";
import {
  evaluateLikelyLocalNativeTaskIntent,
  hasStructuralExecutionIntent,
} from "./executionIntentPolicy";
import { candidateLocalPathsFromText } from "./localPathIntent";

type LocalProductivityAppKind =
  | "calendar"
  | "mail"
  | "reminders"
  | "notes"
  | "contacts"
  | "photos"
  | "music";

const LOCAL_APP_ACTION_PATTERNS: Record<
  LocalProductivityAppKind,
  RegExp[]
> = {
  calendar: [
    /\b(?:check|read|review|search|show|list|find|query|add|create|schedule|book|move|update|cancel|delete)\b[\s\S]{0,96}\b(?:calendar|event|meeting|appointment)\b/i,
    /\b(?:calendar|event|meeting|appointment)\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find|query|add|create|schedule|book|move|update|cancel|delete)\b/i,
  ],
  mail: [
    /\b(?:check|read|review|search|show|list|find|open|add|create|compose|draft|reply|send)\b[\s\S]{0,96}\b(?:mail|email|e-mail|inbox|draft)\b/i,
    /\b(?:mail|email|e-mail|inbox|draft)\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find|open|add|create|compose|draft|reply|send)\b/i,
  ],
  reminders: [
    /\b(?:check|read|review|show|list|find|add|create|set|complete|delete)\b[\s\S]{0,96}\b(?:reminder|reminders|todo|to-do)\b/i,
    /\b(?:reminder|reminders|todo|to-do)\b[\s\S]{0,96}\b(?:check|read|review|show|list|find|add|create|set|complete|delete)\b/i,
  ],
  notes: [
    /\b(?:check|read|review|search|show|list|find|add|create|save|write|delete)\b[\s\S]{0,96}\b(?:apple\s+)?notes?\b/i,
    /\b(?:apple\s+)?notes?\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find|add|create|save|write|delete)\b/i,
  ],
  contacts: [
    /\b(?:check|read|review|search|show|list|find|look\s+up)\b[\s\S]{0,96}\b(?:contacts?|address\s+book)\b/i,
    /\b(?:contacts?|address\s+book)\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find|look\s+up)\b/i,
  ],
  photos: [
    /\b(?:check|read|review|search|show|list|find)\b[\s\S]{0,96}\b(?:photos?|photo\s+library|camera\s+roll)\b/i,
    /\b(?:photos?|photo\s+library|camera\s+roll)\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find)\b/i,
  ],
  music: [
    /\b(?:check|read|review|search|show|list|find)\b[\s\S]{0,96}\b(?:music|songs?|tracks?|playlists?|media\s+library)\b/i,
    /\b(?:music|songs?|tracks?|playlists?|media\s+library)\b[\s\S]{0,96}\b(?:check|read|review|search|show|list|find)\b/i,
  ],
};

/**
 * Direct native-app shortcuts are intentionally single-surface conveniences.
 * Compound work belongs to the planner so one embedded phrase (for example,
 * "create a Mail draft") cannot discard the file, web, or Calendar work that
 * surrounds it.
 */
function hasIndependentNonAppWork(prompt: string) {
  const normalized = prompt.trim();
  if (!normalized) {
    return true;
  }

  const requestedFormats = new Set(
    Array.from(
      normalized.matchAll(/\.(?:csv|docx|html|json|md|pdf|pptx|rtf|txt|xls|xlsx|xml)\b/gi),
      (match) => match[0].toLowerCase(),
    ),
  );
  const hasIndependentFileWork =
    candidateLocalPathsFromText(normalized).length > 0 ||
    requestedFormats.size > 1 ||
    /\b(?:read|inspect|review|analy[sz]e|reconcile|compare|create|generate|produce|deliver|write|save|export)\b[\s\S]{0,120}\b(?:files?|folders?|directories|workbooks?|spreadsheets?|presentations?|documents?|\.csv|\.docx|\.json|\.md|\.pdf|\.pptx|\.txt|\.xlsx)\b/i.test(
      normalized,
    );
  const hasIndependentWebWork =
    /\b(?:research|search|browse|verify|look\s+up)\b[\s\S]{0,120}\b(?:web|internet|online|official|primary\s+sources?|urls?)\b/i.test(
      normalized,
    ) ||
    /\b(?:web|internet|online|official|primary\s+sources?|urls?)\b[\s\S]{0,120}\b(?:research|search|browse|verify|look\s+up)\b/i.test(
      normalized,
    );
  const hasIndependentConnectorWork =
    /\b(?:post|publish|share|upload|notify|message)\b[\s\S]{0,96}\b(?:slack|teams|discord|notion|jira|linear)\b/i.test(
      normalized,
    ) ||
    /\b(?:slack|teams|discord|notion|jira|linear)\b[\s\S]{0,96}\b(?:post|publish|share|upload|notify|message)\b/i.test(
      normalized,
    );
  return (
    hasIndependentFileWork ||
    hasIndependentWebWork ||
    hasIndependentConnectorWork ||
    hasStructuralExecutionIntent(normalized)
  );
}

export function isFocusedLocalAppShortcutRequest(
  prompt: string,
  appKind: LocalProductivityAppKind,
) {
  const normalized = prompt.trim();
  if (!normalized || hasIndependentNonAppWork(normalized)) {
    return false;
  }
  const targetsAnotherLocalApp = Object.entries(LOCAL_APP_ACTION_PATTERNS).some(
    ([kind, patterns]) =>
      kind !== appKind && patterns.some((pattern) => pattern.test(normalized)),
  );
  return !targetsAnotherLocalApp;
}

export function isFocusedLocalAppleUiShortcutRequest(
  prompt: string,
) {
  const normalized = prompt.trim();
  if (!normalized || hasIndependentNonAppWork(normalized)) {
    return false;
  }
  return ![
    targetsLocalCalendar,
    targetsLocalMail,
    targetsLocalReminders,
    targetsLocalNotes,
    targetsLocalContacts,
    targetsLocalPhotos,
    targetsLocalMusic,
  ].some((targetsApp) => targetsApp(normalized));
}

export function hasAmbiguousPrivateAppReadLanguage(message: string) {
  const hasLocalPath = candidateLocalPathsFromText(message).length > 0;
  const explicitAppleApp = Boolean(appleAppNameForExplicitUiIntent(message));
  const explicitPrivateAppTarget =
    [
      targetsLocalCalendar,
      targetsLocalMail,
      targetsLocalReminders,
      targetsLocalNotes,
      targetsLocalContacts,
      targetsLocalPhotos,
      targetsLocalMusic,
    ].some((targetsApp) => targetsApp(message)) ||
    /\b(?:my|our)\b[\s\S]{0,64}\b(?:mail|emails?|inbox|calendar|agenda|events?|reminders?|notes?|contacts?|photos?|music|messages?)\b/i.test(message) ||
    /\b(?:mail|emails?|inbox|calendar|agenda|events?|reminders?|notes?|contacts?|photos?|music|messages?)\b[\s\S]{0,64}\b(?:my|our)\b/i.test(message) ||
    /\bapple\s+(?:mail|calendar|reminders?|notes?|contacts?|photos?|music|messages?)\b/i.test(message) ||
    /\b(?:mail|calendar|reminders?|notes?|contacts?|photos?|music|messages?)\s+(?:app|application)\b/i.test(message) ||
    /\b(?:do|did)\s+i\s+have\b[\s\S]{0,64}\b(?:mail|emails?|events?|reminders?|notes?|contacts?|messages?)\b/i.test(message) ||
    /\bare\s+there\b[\s\S]{0,64}\b(?:unread|new|upcoming|overdue|pending)\b[\s\S]{0,32}\b(?:mail|emails?|events?|reminders?|messages?)\b/i.test(message);
  const ambiguityCandidate = message.replace(
    /\b(?:release|patch|version|changelog)\s+notes?\b/gi,
    "public document",
  );
  const ambiguousPrivateAppRead =
    /\b(?:check|find|list|look\s+(?:at|for|up)|read|review|scan|search|show|summari[sz]e)\b/i.test(ambiguityCandidate) &&
    /\b(?:mail|emails?|inbox|calendar|agenda|events?|reminders?|notes?|contacts?|photos?|music|messages?)\b/i.test(ambiguityCandidate);
  return ambiguousPrivateAppRead && !hasLocalPath && !explicitAppleApp && !explicitPrivateAppTarget;
}

export function evaluateLikelyLocalAppNativeTaskIntent(
  message: string,
  hasInternalMemoryIntent: boolean,
) {
  if (hasAmbiguousPrivateAppReadLanguage(message)) {
    return false;
  }
  const hasLocalPath = candidateLocalPathsFromText(message).length > 0;
  const explicitAppleApp = Boolean(appleAppNameForExplicitUiIntent(message));
  const explicitPrivateAppTarget = Boolean(readOnlyPrivateAppToolForPrompt(message));
  return evaluateLikelyLocalNativeTaskIntent(message, {
    hasInternalMemoryIntent,
    hasLocalPath,
    mentionsAppleApp: explicitAppleApp || explicitPrivateAppTarget,
  });
}

export function localProductivityAppKindForTool(
  toolName: string,
): LocalProductivityAppKind | null {
  switch (toolName) {
    case "read_system_calendar":
      return "calendar";
    case "draft_system_email":
    case "read_system_emails":
      return "mail";
    case "add_system_reminder":
    case "read_system_reminders":
      return "reminders";
    case "create_system_note":
    case "read_system_notes":
      return "notes";
    case "read_system_contacts":
      return "contacts";
    case "read_system_photos":
      return "photos";
    case "read_system_music":
      return "music";
    default:
      return null;
  }
}

function hasStrongPrivateAppFallbackEvidence(
  prompt: string,
  appKind: LocalProductivityAppKind,
) {
  const normalized = prompt.trim().toLowerCase();
  const hasAction =
    /\b(check|find|look\s+(?:at|for|up|through)|lookup|read|review|scan|search|show|list|see|summari[sz]e|report|open|add|create|write|draft|compose|what|which|who|when)\b/i.test(
      normalized,
    ) || /\b(?:do|did|have)\s+i\b/i.test(normalized);
  if (!hasAction) {
    return false;
  }
  const hasPersonalScope =
    /\b(?:my|our)\b/i.test(normalized) ||
    /\b(?:do|did|have)\s+i\b/i.test(normalized) ||
    /\bfor\s+me\b/i.test(normalized);
  const targetedAppKind = localProductivityAppKindForTool(
    readOnlyPrivateAppToolForPrompt(normalized) ?? "",
  );
  const explicitNotesDestination =
    appKind === "notes" && explicitlyTargetsLocalNotesApp(normalized);
  const explicitLocations: Record<LocalProductivityAppKind, string[]> = {
    calendar: ["calendar app", "apple calendar", "macos calendar", "google calendar"],
    mail: ["mail app", "apple mail", "mail draft", "gmail inbox", "outlook inbox"],
    reminders: ["reminders app", "apple reminders"],
    notes: ["notes app", "apple notes"],
    contacts: ["contacts app", "apple contacts", "address book"],
    photos: ["photos app", "apple photos", "icloud photos", "photo library", "camera roll"],
    music: ["music app", "apple music library", "music library", "media library"],
  };

  return (
    hasPersonalScope ||
    targetedAppKind === appKind ||
    explicitNotesDestination ||
    explicitLocations[appKind].some((location) => normalized.includes(location))
  );
}

export async function retainApprovedLocalAppRequest<T>(
  request: T | null,
  prompt: string,
  appKind: LocalProductivityAppKind | null,
  onAmbiguousTriageFailure?: () => void,
) {
  if (!request || !appKind) {
    return request;
  }
  if (!isFocusedLocalAppShortcutRequest(prompt, appKind)) {
    return null;
  }
  if (!isTauriRuntime) {
    return request;
  }
  try {
    const approved = await invoke<boolean>("triage_local_app_intent", {
      prompt,
      appKind,
    });
    return approved ? request : null;
  } catch {
    if (hasStrongPrivateAppFallbackEvidence(prompt, appKind)) {
      return request;
    }
    onAmbiguousTriageFailure?.();
    return null;
  }
}

function mentionsLocalSystemTopic(message: string) {
  return [
    "mail",
    "email",
    "e-mail",
    "inbox",
    "calendar",
    "agenda",
    "schedule",
    "reminder",
    "task",
    "todo",
    "to-do",
    "note",
    "contact",
    "address book",
    "apple music",
    "music library",
    "media library",
    "messages app",
    "imessage",
  ].some((term) => message.includes(term));
}

export function isInformationalLocalSystemTopicQuestion(message: string) {
  const normalized = message.trim().toLowerCase();
  if (!normalized || !mentionsLocalSystemTopic(normalized)) {
    return false;
  }

  if (isNaturalMailboxStateQuestion(normalized)) {
    return false;
  }

  if (
    [
      "check my",
      "read my",
      "review my",
      "scan my",
      "show my",
      "show me my",
      "list my",
      "find my",
      "look for",
      "summarize my",
      "summarise my",
      "report on my",
      "what is on my",
      "what's on my",
      "what are my",
      "what is in my",
      "what's in my",
      "what is the newest",
      "what's the newest",
      "which song",
      "do i have",
      "did i have",
      "are there",
      "when is my",
      "when are my",
    ].some((term) => normalized.includes(term))
  ) {
    return false;
  }

  return [
    "how do i",
    "how can i",
    "how should i",
    "how does",
    "how do",
    "how ",
    "what is",
    "what are",
    "why does",
    "why do",
    "explain",
    "tell me about",
    "tell me how",
    "help me understand",
    "configure",
    "set up",
    "setup",
    "troubleshoot",
  ].some((term) => normalized.includes(term));
}

const explicitAppleMessagesAppPattern =
  /\b(?:messages?\s+(?:app|application)|apple\s+messages?|imessages?)\b/i;
const genericAppleUiAppSuffixPattern =
  /\b(app\s+store|books|facetime|find\s+my|freeform|home|keychain\s+access|maps|music|news|photos|podcasts|safari|shortcuts|stocks|system\s+settings|tv|weather)\s+(?:app|application)\b/i;
const appleBrandedUiAppPattern =
  /\bapple\s+(app\s+store|books|facetime|find\s+my|freeform|home|keychain\s+access|maps|music|news|photos|podcasts|safari|shortcuts|stocks|system\s+settings|tv|weather)\b/i;

const appleAppUiDescriptors: Array<[string, string]> = [
  ["App Store", "app\\s+store"],
  ["Books", "books"],
  ["Calendar", "calendar"],
  ["Contacts", "contacts"],
  ["FaceTime", "facetime"],
  ["Find My", "find\\s+my"],
  ["Freeform", "freeform"],
  ["Home", "home"],
  ["Keychain Access", "keychain\\s+access"],
  ["Mail", "mail"],
  ["Maps", "maps"],
  ["Messages", "messages"],
  ["Music", "music"],
  ["News", "news"],
  ["Notes", "notes"],
  ["Photos", "photos"],
  ["Podcasts", "podcasts"],
  ["Reminders", "reminders"],
  ["Safari", "safari"],
  ["Shortcuts", "shortcuts"],
  ["Stocks", "stocks"],
  ["System Settings", "system\\s+settings"],
  ["TV", "tv"],
  ["Weather", "weather"],
];

export function appleAppNameForExplicitUiIntent(message: string) {
  for (const [appName, namePattern] of appleAppUiDescriptors) {
    const launchPattern = new RegExp(
      `\\b(?:open|launch|activate|bring\\s+up|switch\\s+to)\\s+(?:the\\s+)?(?:apple\\s+)?${namePattern}(?:\\s+app)?\\s*[.!?]?$`,
      "i",
    );
    if (launchPattern.test(message)) {
      return appName;
    }
  }

  const explicitlyRequestsVisibleAppSurface =
    /\b(?:app|application|visible\s+(?:ui|interface|window|screen|content)|app\s+ui|user\s+interface|window|screen)\b/i.test(
      message,
    );
  const matches = new Set<string>();
  for (const [appName, namePattern] of appleAppUiDescriptors) {
    const referencePattern = appName === "Messages"
      ? explicitAppleMessagesAppPattern
      : new RegExp(
          `\\b(?:apple\\s+${namePattern}|${namePattern}\\s+(?:app|application))\\b`,
          "i",
        );
    if (
      referencePattern.test(message) &&
      (appName === "Messages" || explicitlyRequestsVisibleAppSurface)
    ) {
      matches.add(appName);
    }
  }
  return matches.size === 1 ? [...matches][0] : null;
}

function explicitlyTargetedGenericAppleUiAppNames(message: string) {
  const names = new Set<string>();
  for (const match of message.matchAll(
    new RegExp(genericAppleUiAppSuffixPattern.source, "gi"),
  )) {
    names.add(match[1].toLowerCase());
  }
  const hasSurfaceIntent =
    /\b(?:open|launch|activate|bring\s+up|switch\s+to|app|application|visible\s+(?:ui|interface|window|screen|content)|app\s+ui|user\s+interface|window|screen)\b/i.test(
      message,
    );
  if (hasSurfaceIntent) {
    for (const match of message.matchAll(
      new RegExp(appleBrandedUiAppPattern.source, "gi"),
    )) {
      names.add(match[1].toLowerCase());
    }
  }
  return names;
}

export function explicitlyTargetsGenericAppleUiApp(message: string) {
  return explicitlyTargetedGenericAppleUiAppNames(message).size > 0;
}

export function isNaturalMailboxStateQuestion(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    /\b(?:do\s+i\s+have|are\s+there)\s+(?:any\s+|some\s+)?(?:new\s+|unread\s+|recent\s+)+(?:mail|emails?|e-mails?)\b/i.test(
      normalized,
    ) ||
    /\bhow\s+many\s+(?:new\s+|unread\s+|recent\s+)+(?:mail|emails?|e-mails?)\s+(?:do\s+i\s+have|are\s+(?:there\s+)?(?:in|inside)\s+my\s+inbox)\b/i.test(
      normalized,
    )
  );
}

function readOnlyPrivateAppToolsForPrompt(message: string) {
  const normalized = message.trim().toLowerCase();
  const asksToRead =
    /\b(check|find|look\s+(?:at|for|up|through)|read|review|scan|search|show|list|see|summari[sz]e|summary|report|what|which|who|when|how\s+many)\b/i.test(
      normalized,
    ) ||
    /\b(?:do|did)\s+i\s+have\b|\bare\s+there\b|\b(?:unread|upcoming|overdue|pending)\b/i.test(
      normalized,
    );
  if (!asksToRead) {
    return { ambiguousAppleUiTarget: false, matches: new Set<string>() };
  }

  const matches = new Set<string>();
  const genericAppleUiTargets = explicitlyTargetedGenericAppleUiAppNames(normalized);
  const targets: Array<[string, (value: string) => boolean]> = [
    [
      "read_apple_app_ui",
      (value) =>
        explicitlyTargetsAppleMessagesApp(value) ||
        explicitlyTargetsGenericAppleUiApp(value),
    ],
    ["read_system_calendar", targetsLocalCalendar],
    ["read_system_reminders", targetsLocalReminders],
    ["read_system_notes", targetsLocalNotes],
    ["read_system_contacts", targetsLocalContacts],
    ["read_system_photos", targetsLocalPhotos],
    ["read_system_music", targetsLocalMusic],
    ["read_system_emails", targetsLocalMail],
  ];
  for (const [toolName, targetsApp] of targets) {
    if (targetsApp(normalized)) {
      matches.add(toolName);
    }
  }
  return {
    ambiguousAppleUiTarget: genericAppleUiTargets.size > 1,
    matches,
  };
}

export function readOnlyPrivateAppToolForPrompt(message: string) {
  const { ambiguousAppleUiTarget, matches } = readOnlyPrivateAppToolsForPrompt(message);
  return !ambiguousAppleUiTarget && matches.size === 1 ? [...matches][0] : null;
}

export function hasCompetingReadOnlyPrivateAppTarget(
  message: string,
  expectedToolName: string,
) {
  const { ambiguousAppleUiTarget, matches } = readOnlyPrivateAppToolsForPrompt(message);
  return ambiguousAppleUiTarget || [...matches].some(
    (toolName) => toolName !== expectedToolName,
  );
}

export function hasPrivateAppMutationIntent(message: string) {
  const normalized = message.trim().toLowerCase();
  const mutationPattern =
    /\b(?:archiv(?:e|es|ed|ing)|delet(?:e|es|ed|ing)|trash(?:es|ed|ing)?|mov(?:e|es|ed|ing)|forward(?:s|ed|ing)?|flag(?:s|ged|ging)?|star(?:s|red|ring)?|label(?:s|ed|ing)?|tag(?:s|ged|ging)?|block(?:s|ed|ing)?|mut(?:e|es|ed|ing)|junk(?:s|ed|ing)?|spam(?:s|med|ming)?|mark(?:\s+\w+){0,3}\s+(?:as\s+)?(?:un)?read|send|sends|sending|sent)\b/gi;
  const clauseBoundary = /[.!?;]|\b(?:but|then|and\s+then|after\s+that)\b/gi;

  for (const match of normalized.matchAll(mutationPattern)) {
    const matchIndex = match.index ?? 0;
    let clauseStart = 0;
    clauseBoundary.lastIndex = 0;
    for (const boundary of normalized.slice(0, matchIndex).matchAll(clauseBoundary)) {
      clauseStart = (boundary.index ?? 0) + boundary[0].length;
    }
    const prefix = normalized.slice(clauseStart, matchIndex);
    const suffix = normalized.slice(matchIndex + match[0].length);
    const isNegated =
      /\b(?:do\s+not|don't|dont|never|without|not\s+to)(?:\s+\w+){0,5}\s*$/i.test(
        prefix,
      ) || /^\s+nothing\b/i.test(suffix);
    if (!isNegated) {
      return true;
    }
  }
  return false;
}

export function explicitlyTargetsAppleMessagesApp(message: string) {
  return explicitAppleMessagesAppPattern.test(message);
}

function hasOomuTaskContext(message: string) {
  return (
    /\btasks?\b/i.test(message) &&
    (/\b(?:projects?|workflows?|oomu)\b/i.test(message) ||
      /\btasks?\s+(?:screen|tab|view|panel|page)\b/i.test(message))
  );
}

export function targetsLocalReminders(message: string) {
  const normalized = message.trim().toLowerCase();
  const explicitApp = /\b(?:apple\s+reminders?|reminders?\s+(?:app|application))\b/i.test(
    normalized,
  );
  const personalReminder =
    /\b(?:my|our)\s+(?:open\s+|pending\s+|outstanding\s+)?(?:reminders?|todos?|to-dos?|tasks?)\b/i.test(
      normalized,
    ) ||
    /\b(?:do|did)\s+i\s+have\b[\s\S]{0,48}\b(?:reminders?|todos?|to-dos?|tasks?)\b/i.test(
      normalized,
    );
  const directReminderAction =
    /\bremind\s+me\b/i.test(normalized) ||
    /\b(?:add|create|make|set)\b[\s\S]{0,40}\b(?:reminders?|todos?|to-dos?|to\s+do)\b/i.test(
      normalized,
    ) ||
    /\b(?:reminders?|todos?|to-dos?|to\s+do)\b[\s\S]{0,40}\b(?:add|create|make|set)\b/i.test(
      normalized,
    );

  if (explicitApp) {
    return true;
  }
  return !hasOomuTaskContext(normalized) && (personalReminder || directReminderAction);
}

export function targetsLocalCalendar(message: string) {
  const normalized = message.trim().toLowerCase();
  const directlyChecksCalendar =
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:check|read|review|show|list|find|search|scan|look\s+(?:at|for|up))\s+(?:the\s+)?calendars?(?:(?:\s+(?:for|on|from|between|during|today|tomorrow|this|next|upcoming)\b[\s\S]*)|\s*[.!?]?)$/i.test(
      normalized,
    );
  return (
    /\b(?:apple\s+calendar|macos\s+calendar|calendar\s+(?:app|application))\b/i.test(
      normalized,
    ) ||
    /\b(?:my|our)\s+(?:calendar|calendars|agenda|schedule|events?|meetings?|appointments?)\b/i.test(
      normalized,
    ) ||
    /\b(?:do|did)\s+i\s+have\b[\s\S]{0,64}\b(?:events?|meetings?|appointments?)\b/i.test(
      normalized,
    ) ||
    directlyChecksCalendar
  );
}

export function targetsLocalMail(message: string) {
  const normalized = message.trim().toLowerCase();
  const directlyChecksMailbox =
    /^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:check|read|review|show|list|find|search|scan|look\s+(?:at|for|up))\s+(?:the\s+)?(?:(?:new|unread|recent)\s+)?(?:mail|emails?|e-mails?|inbox)(?:(?:\s+(?:for|from|sent|received|about|with|dated|today|this|last|new|unread|recent|and|but)\b[\s\S]*)|\s*[.!?]?)$/i.test(
      normalized,
    );
  return (
    /\b(?:apple\s+mail|mail\s+(?:app|application)|gmail\s+inbox|outlook\s+inbox)\b/i.test(
      normalized,
    ) ||
    /\b(?:my|our)\s+(?:new\s+|unread\s+|recent\s+)?(?:mail|emails?|e-mails?|inbox)\b/i.test(
      normalized,
    ) ||
    isNaturalMailboxStateQuestion(normalized) ||
    directlyChecksMailbox
  );
}

export function targetsLocalNotes(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    explicitlyTargetsLocalNotesApp(normalized) ||
    /\b(?:my|our)\s+(?:recent\s+|saved\s+)?notes?\b/i.test(normalized) ||
    /\bnotes?\s+(?:i|we)\s+(?:saved|created|wrote|have)\b/i.test(normalized)
  );
}

export function explicitlyTargetsLocalNotesApp(message: string) {
  return (
    /\b(?:apple\s+notes|notes?\s+(?:app|application))\b/i.test(message) ||
    /\b(?:in|into|to|inside|within)\s+(?:(?:my|the)\s+)?(?:apple\s+)?notes?(?:\s+(?:app|application))?\b/i.test(
      message,
    )
  );
}

export function targetsLocalContacts(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    /\b(?:apple\s+contacts|contacts?\s+(?:app|application)|address\s+book)\b/i.test(
      normalized,
    ) || /\b(?:my|our)\s+contacts?\b/i.test(normalized)
  );
}

export function targetsLocalPhotos(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    /\b(?:apple\s+photos|photos?\s+(?:app|application)|photo\s+library|camera\s+roll)\b/i.test(
      normalized,
    ) ||
    /\b(?:my|our)\s+(?:newest\s+|latest\s+|recent\s+)?(?:photos?|photo\s+albums?|images?)\b/i.test(
      normalized,
    )
  );
}

export function targetsLocalMusic(message: string) {
  const normalized = message.trim().toLowerCase();
  return (
    /\b(?:apple\s+music\s+library|music\s+(?:app|application|library)|media\s+library)\b/i.test(
      normalized,
    ) ||
    /\b(?:my|our)\s+(?:(?:newest|latest|most\s+recent|recently\s+added|last\s+added)\s+)?(?:music|songs?|tracks?|playlists?)\b/i.test(
      normalized,
    ) ||
    /\b(?:add|added|save|saved)\b[\s\S]{0,64}\b(?:to|in)\s+apple\s+music\b/i.test(
      normalized,
    )
  );
}
