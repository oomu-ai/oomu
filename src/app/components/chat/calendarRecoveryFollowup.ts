import {
  agentRecoveryActionKey,
} from "./agentExecutionRecovery";
import {
  parseAgentExecutionRecoveryReceipt,
} from "./RecoveryReceiptCard";
import { stableErrorCode } from "./inferenceErrors";

type RecoveryTranscriptMessage = {
  role: "user" | "assistant" | "system";
  content: string;
};

type CalendarRecoveryFollowup = {
  calendarName: string;
  executionId: string;
};

type ResolveCalendarRecovery = (
  executionId: string,
  choice: { resolution: "select_existing"; calendarName: string },
) => Promise<"cancelled" | "resumed">;

type CalendarRecoveryFollowupOutcome = {
  contentKey:
    | "chat.recovery.calendar_ambiguous_body"
    | "chat.recovery.calendar_failed"
    | "chat.recovery.calendar_incompatible_body"
    | "chat.recovery.calendar_missing_body"
    | "chat.recovery.calendar_read_only_body"
    | "chat.recovery.calendar_resolved";
  contentVariables?: { calendar: string };
  role: "assistant" | "system";
  status: "completed" | "failed";
  statusKey: "chat.recovery.calendar_continuing" | "chat.status.halted";
};

const calendarFollowupPattern =
  /^use\s+my\s+([^\r\n]+?)\s+calendar\s+instead\s+and\s+continue[.!]?$/iu;

function unquoteCalendarName(value: string) {
  const pairs: ReadonlyArray<readonly [string, string]> = [
    ['"', '"'],
    ["'", "'"],
    ["“", "”"],
    ["‘", "’"],
  ];
  const pair = pairs.find(([open, close]) => value.startsWith(open) && value.endsWith(close));
  return pair ? value.slice(pair[0].length, -pair[1].length).trim() : value;
}

export function calendarNameFromRecoveryFollowup(message: string) {
  const match = calendarFollowupPattern.exec(message.trim());
  if (!match) return null;
  const calendarName = unquoteCalendarName(match[1].trim());
  if (
    !calendarName ||
    Array.from(calendarName).length > 80 ||
    /[\u0000-\u001f\u007f]/u.test(calendarName)
  ) {
    return null;
  }
  return calendarName;
}

export function calendarRecoveryFollowupForTranscript(
  message: string,
  transcript: readonly RecoveryTranscriptMessage[],
  completedActionKeys: ReadonlySet<string> = new Set(),
): CalendarRecoveryFollowup | null {
  const calendarName = calendarNameFromRecoveryFollowup(message);
  if (!calendarName) return null;

  // The newest durable recovery receipt is authoritative. An older calendar
  // pause must never capture a correction intended for newer stopped work.
  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const entry = transcript[index];
    if (entry.role !== "assistant") continue;
    const receipt = parseAgentExecutionRecoveryReceipt(entry.content);
    if (!receipt) continue;
    if (
      receipt.recoveryAction !== "resolve_calendar_target" ||
      completedActionKeys.has(
        agentRecoveryActionKey(receipt.executionId, "resolve_calendar_target"),
      )
    ) {
      return null;
    }
    return { calendarName, executionId: receipt.executionId };
  }
  return null;
}

export async function resolveCalendarRecoveryFollowup(
  followup: CalendarRecoveryFollowup,
  resolve: ResolveCalendarRecovery,
): Promise<CalendarRecoveryFollowupOutcome> {
  try {
    const outcome = await resolve(followup.executionId, {
      resolution: "select_existing",
      calendarName: followup.calendarName,
    });
    if (outcome !== "resumed") throw new Error("calendar_recovery_cancelled_unexpectedly");
    return {
      contentKey: "chat.recovery.calendar_resolved",
      role: "assistant",
      status: "completed",
      statusKey: "chat.recovery.calendar_continuing",
    };
  } catch (error) {
    const failureKey = (() => {
      switch (stableErrorCode(error)) {
        case "calendar_not_found":
          return "chat.recovery.calendar_missing_body" as const;
        case "calendar_name_ambiguous":
          return "chat.recovery.calendar_ambiguous_body" as const;
        case "calendar_read_only":
          return "chat.recovery.calendar_read_only_body" as const;
        case "calendar_availability_unsupported":
          return "chat.recovery.calendar_incompatible_body" as const;
        default:
          return "chat.recovery.calendar_failed" as const;
      }
    })();
    return {
      contentKey: failureKey,
      ...(failureKey === "chat.recovery.calendar_failed"
        ? {}
        : { contentVariables: { calendar: followup.calendarName } }),
      role: "system",
      status: "failed",
      statusKey: "chat.status.halted",
    };
  }
}
