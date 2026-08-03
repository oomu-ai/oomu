import type { AppSection } from "@/components/AppShell";
import type { RoutineDraft, RoutineHandoffRequest } from "./routineDraft";
import { plannedRoutineWorkflowAttachment } from "./routineTargetWorkflow";

export function openRoutineReview(
  request: RoutineHandoffRequest,
  projectId: string | null,
  setDraft: (draft: RoutineDraft | null) => void,
  navigate: (item: AppSection) => void,
) {
  const id = crypto.randomUUID();
  setDraft({
    id,
    ...request,
    workflowAttachment: plannedRoutineWorkflowAttachment(request, id, projectId),
  });
  navigate("routines");
}
