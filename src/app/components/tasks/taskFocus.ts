import { TASK_STATES, type TaskState } from "@/lib/p0Contracts";

const TASK_FOCUS_KEY = "oomu.tasks.focus";
const TASK_RUN_ID_PATTERN = /^[A-Za-z0-9_-]{1,200}$/;

type TaskFocusTarget = {
  taskRunId: string;
  state: TaskState | null;
};

function isTaskState(value: unknown): value is TaskState {
  return typeof value === "string" && TASK_STATES.some((state) => state === value);
}

function parseTaskFocusTarget(value: string | null): TaskFocusTarget | null {
  if (!value) return null;

  // Preserve handoffs made by older builds. They did not carry state, so the
  // Task center will use its all-results fallback rather than hiding the target.
  if (TASK_RUN_ID_PATTERN.test(value)) {
    return { taskRunId: value, state: null };
  }

  try {
    const parsed = JSON.parse(value) as Record<string, unknown>;
    if (!TASK_RUN_ID_PATTERN.test(String(parsed.taskRunId ?? ""))) return null;
    return {
      taskRunId: String(parsed.taskRunId),
      state: isTaskState(parsed.state) ? parsed.state : null,
    };
  } catch {
    return null;
  }
}

export function requestTaskFocus(taskRunId: string, state?: string) {
  if (typeof window === "undefined" || !TASK_RUN_ID_PATTERN.test(taskRunId)) return;
  const target: TaskFocusTarget = {
    taskRunId,
    state: isTaskState(state) ? state : null,
  };
  window.sessionStorage.setItem(TASK_FOCUS_KEY, JSON.stringify(target));
}

export function peekTaskFocus(): TaskFocusTarget | null {
  if (typeof window === "undefined") return null;
  return parseTaskFocusTarget(window.sessionStorage.getItem(TASK_FOCUS_KEY));
}

export function consumeTaskFocus(): TaskFocusTarget | null {
  if (typeof window === "undefined") return null;
  const target = peekTaskFocus();
  window.sessionStorage.removeItem(TASK_FOCUS_KEY);
  return target;
}
