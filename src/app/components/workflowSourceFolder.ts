import { invoke } from "@/lib/invoke";

export type WorkflowSourceFolder = {
  fileCount: number;
  folderName: string;
  folderPath: string;
  totalBytes: number;
  truncated: boolean;
};

export function chooseWorkflowSourceFolder({
  title,
  truncationNotice,
}: {
  title: string;
  truncationNotice: string;
}) {
  return invoke<WorkflowSourceFolder | null>("choose_workflow_source_folder", {
    selectionId: newWorkflowSelectionId(),
    title,
    truncationNotice,
  });
}

function newWorkflowSelectionId() {
  const id =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return `selection-${id}`;
}
