export type RoutineCadenceUnit =
  | "minute"
  | "hour"
  | "day"
  | "week"
  | "month"
  | "quarter"
  | "year";

export type RoutineCadence = {
  interval: number;
  unit: RoutineCadenceUnit;
};

export type RoutineTargetAction = {
  kind: "read_unread_mail";
};

export type RoutineWorkflowAttachment = {
  projectPlanned: boolean;
  projectId: string;
  workflowIr?: import("../workflowIr").WorkflowIr;
  workflowId: string;
  workflowName: string;
  workflowVersion: number;
};

export type RoutineHandoffRequest = {
  requestText: string;
  scheduleText: string;
  scheduleKind: "one_shot" | "recurring";
  cadence: RoutineCadence | null;
  scheduleSupported: boolean;
  timingDefaulted: boolean;
  cadenceBoundaryConflict: boolean;
  runOnceRequested: boolean;
  endBoundary: "midnight" | null;
  targetAction?: RoutineTargetAction;
};

export type RoutineDraft = RoutineHandoffRequest & {
  id: string;
  workflowAttachment?: RoutineWorkflowAttachment;
};
