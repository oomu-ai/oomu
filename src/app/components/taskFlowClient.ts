"use client";

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@/lib/invoke";

type TaskFlowStatus =
  | "queued"
  | "active"
  | "verified"
  | "failed"
  | "diagnostic"
  | "paused"
  | "secure_pause"
  | "cancelled";

type TaskFlowStepStatus =
  | "queued"
  | "active"
  | "verified"
  | "failed"
  | "skipped"
  | "cancelled";

type TaskFlowStep = {
  step_id: string;
  sequence: number;
  status: TaskFlowStepStatus;
  pre_conditions: string[];
  action: unknown;
  post_conditions: string[];
  logical_certificate?: unknown | null;
  output?: string | null;
  decision_node?: string | null;
};

type TaskHeartbeat = {
  id: number;
  flow_id: string;
  step_id?: string | null;
  parent_session_id: string;
  status: string;
  drift_score: number;
  message: string;
  created_at_ms: number;
};

type DecisionNode = {
  id: number;
  flow_id: string;
  failed_step_id: string;
  reason: string;
  suggested_fix: string;
  status: string;
  created_at_ms: number;
};

export type TaskFlow = {
  flow_id: string;
  mission_id: string;
  parent_session_id: string;
  directive: string;
  status: TaskFlowStatus;
  steps: TaskFlowStep[];
  decision_nodes: DecisionNode[];
  heartbeats: TaskHeartbeat[];
  created_at_ms: number;
  updated_at_ms: number;
};

export type TaskFlowExecutionResponse = {
  flow: TaskFlow;
  completed_steps: number;
  halted: boolean;
  diagnostic?: DecisionNode | null;
};

export function taskFlowExecutionIsVerified(response: TaskFlowExecutionResponse) {
  return (
    !response.halted &&
    response.flow.status === "verified" &&
    response.completed_steps === response.flow.steps.length &&
    response.flow.steps.every((step) => step.status === "verified")
  );
}

type TaskFlowProgressEvent = {
  flow_id: string;
  mission_id: string;
  parent_session_id: string;
  step_id?: string | null;
  step_index: number;
  status: TaskFlowStepStatus | TaskFlowStatus;
  message: string;
};

type TaskFlowThoughtEvent = {
  flow_id: string;
  mission_id: string;
  parent_session_id: string;
  step_id: string;
  step_index: number;
  phase: string;
  thought: string;
};

type TaskFlowTurnContextRequest = {
  turn_id: string;
  generation_token: string;
  session_id: string;
  agent_id: string;
  provider_id: string;
  model_id: string;
  parent_turn_id: string | null;
  root_turn_id: string;
  turn_kind: "root" | "queued" | "steer" | "retry";
};

type CreateTaskFlowRequest = TaskFlowTurnContextRequest & {
  directive: string;
  parent_session_id: string;
};

type TaskFlowSubscription = {
  flowId?: string;
  parentSessionId?: string;
  onProgress?: (event: TaskFlowProgressEvent) => void;
  onThought?: (event: TaskFlowThoughtEvent) => void;
};

export async function createTaskFlow(request: CreateTaskFlowRequest) {
  return invoke<TaskFlow>("create_taskflow", { request });
}

export async function executeTaskFlow(flowId: string, turnContext: TaskFlowTurnContextRequest) {
  return invoke<TaskFlowExecutionResponse>("execute_taskflow", {
    request: { flow_id: flowId, ...turnContext },
  });
}

export function buildChatTaskFlowDirective(message: string, attachments: { name: string; text?: string }[]) {
  const attachmentContext = attachments
    .filter((attachment) => attachment.text?.trim())
    .map((attachment) => `Attachment: ${attachment.name}\n${attachment.text?.trim()}`)
    .join("\n\n");
  return [
    "Execute this chat-delegated TaskFlow as a local multi-step task.",
    message.trim(),
    attachmentContext,
  ].filter(Boolean).join("\n\n");
}

export async function subscribeToTaskFlowEvents({
  flowId,
  parentSessionId,
  onProgress,
  onThought,
}: TaskFlowSubscription) {
  try {
    const unlistenProgress = await listen<TaskFlowProgressEvent>(
      "taskflow://progress",
      (event) => {
        if (!matchesTaskFlowScope(event.payload, flowId, parentSessionId)) {
          return;
        }
        onProgress?.(event.payload);
      },
    );
    const unlistenThought = await listen<TaskFlowThoughtEvent>(
      "taskflow://thought",
      (event) => {
        if (!matchesTaskFlowScope(event.payload, flowId, parentSessionId)) {
          return;
        }
        onThought?.(event.payload);
      },
    );
    return () => {
      unlistenProgress();
      unlistenThought();
    };
  } catch {
    return () => undefined;
  }
}

export function taskFlowChatLine(event: TaskFlowProgressEvent | TaskFlowThoughtEvent) {
  if ("thought" in event) {
    return `- ${event.step_id}: ${event.phase} - ${event.thought}`;
  }
  const step = event.step_id ? `${event.step_id}: ` : "";
  return `- ${step}${event.status} - ${event.message}`;
}

function matchesTaskFlowScope(
  event: { flow_id: string; parent_session_id: string },
  flowId?: string,
  parentSessionId?: string,
) {
  if (flowId && event.flow_id !== flowId) {
    return false;
  }
  if (parentSessionId && event.parent_session_id !== parentSessionId) {
    return false;
  }
  return true;
}
