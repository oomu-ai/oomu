import { invoke } from "@/lib/invoke";

export type AppControlControl =
  | "pause"
  | "stop"
  | "take_control"
  | "return_to_oomu";

export type AppControlState =
  | "observing"
  | "running"
  | "paused"
  | "takeover"
  | "return_pending"
  | "completed"
  | "stopped"
  | "failed";

export type AppControlActionKind =
  | "focus"
  | "press"
  | "select"
  | "type_text"
  | "invoke_menu"
  | "scroll"
  | "drag_drop"
  | "choose_file"
  | "apple_event";

export type AppControlPauseReason =
  | "user_input"
  | "secure_field"
  | "ambiguous_target"
  | "repeated_mismatch"
  | "unexpected_navigation"
  | "permission_changed"
  | "hidden_window"
  | "application_changed"
  | "driver_unavailable";

export type AppControlOutcomeStatus =
  | "verified"
  | "no_change"
  | "failed"
  | "paused";

export type AppControlIcon =
  | "finder"
  | "preview"
  | "mail"
  | "calendar"
  | "numbers"
  | "keynote"
  | "excel"
  | "powerpoint"
  | "generic";

export interface AppControlSessionView {
  sessionId: string;
  taskRunId: string;
  projectId: string;
  state: AppControlState;
  application?: {
    name: string;
    icon: AppControlIcon;
  } | null;
  currentAction?: {
    kind: AppControlActionKind;
    targetLabel: string | null;
    willChangeData: boolean;
  } | null;
  pauseReason?: AppControlPauseReason | null;
  canPause: boolean;
  canTakeControl: boolean;
  canReturnToOomu: boolean;
  observationGeneration: number;
  lastOutcome?: {
    status: AppControlOutcomeStatus;
    actionKind: AppControlActionKind;
    receiptId: string;
    recordedAtMs: number;
    detailsAvailable: boolean;
  } | null;
  updatedAtMs: number;
}

export const appControlApi = {
  getStatus: (taskRunId?: string | null) =>
    invoke<AppControlSessionView | null>("get_app_control_status", {
      request: { taskRunId: optionalValue(taskRunId) },
    }),

  control: (
    sessionId: string,
    taskRunId: string,
    control: AppControlControl,
  ) =>
    invoke<AppControlSessionView>("control_app_control_session", {
      request: {
        sessionId: requiredValue(sessionId),
        taskRunId: requiredValue(taskRunId),
        control,
      },
    }),
};

function requiredValue(value: string) {
  const normalized = value.trim();
  if (!normalized) throw new Error("app_control_identifier_required");
  return normalized;
}

function optionalValue(value?: string | null) {
  const normalized = value?.trim();
  return normalized || null;
}
