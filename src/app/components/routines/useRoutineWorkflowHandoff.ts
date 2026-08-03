import { useCallback, useEffect, useMemo, useState } from "react";
import type { ProjectRecord } from "../projects/projectClient";
import type { RoutineDraft } from "./routineDraft";
import { composeRoutineTargetWorkflow } from "./routineTargetWorkflow";
import {
  workflowReviewCapabilities,
  type WorkflowReviewCapabilities,
} from "./workflowReviewCapabilities";

export type RoutineWorkflowOption = {
  id: string;
  name: string;
  steps?: string;
  description?: string;
  workflowVersion?: number;
  version?: number;
  projectId?: string | null;
  reviewCapabilities?: WorkflowReviewCapabilities;
  compilationStatus?: "Draft" | "Compiling" | "Compiled" | "Failed";
};

type RoutineTranslate = (key: string) => string;

const unavailableReview: WorkflowReviewCapabilities = {
  status: "unavailable",
  calendarCreate: false,
  calendarRead: false,
  emailDraft: false,
  emailRead: false,
  emailSend: false,
  officialWeb: false,
  projectFileRead: false,
  projectFileWrite: false,
};

export function useRoutineWorkflowHandoff(
  projects: ProjectRecord[],
  workflows: RoutineWorkflowOption[],
  t: RoutineTranslate,
) {
  const [draft, setDraft] = useState<RoutineDraft | null>(null);
  const [preparationBusy, setPreparationBusy] = useState(false);
  const [preparationFailed, setPreparationFailed] = useState(false);
  const attachment = draft?.workflowAttachment;
  const preparationRequired = Boolean(attachment && !attachment.workflowIr);

  const reviewProjects = useMemo(() => {
    if (!attachment?.projectPlanned) return projects;
    const planned: ProjectRecord = {
      projectId: attachment.projectId as ProjectRecord["projectId"],
      name: t("routines.handoff_project_name"),
      description: t("routines.handoff_project_description"),
      dataPolicy: "local_only",
      instructions: "",
      archivedAtMs: null,
      createdAtMs: 0,
      updatedAtMs: 0,
      sourceCount: 0,
      conversationCount: 0,
      workflowCount: 0,
      taskCount: 0,
    };
    return [planned, ...projects];
  }, [attachment, projects, t]);

  const reviewWorkflows = useMemo(() => {
    if (!attachment || workflows.some((item) => item.id === attachment.workflowId)) {
      return workflows;
    }
    const reviewCapabilities = attachment.workflowIr
      ? workflowReviewCapabilities(
          JSON.stringify({ workflowIr: attachment.workflowIr }),
        )
      : unavailableReview;
    const planned: RoutineWorkflowOption = {
      id: attachment.workflowId,
      name:
        attachment.workflowIr?.name || t("routines.handoff_workflow_name"),
      description:
        attachment.workflowIr?.description ||
        t("routines.handoff_workflow_description"),
      workflowVersion: attachment.workflowVersion,
      projectId: attachment.projectId,
      steps: attachment.workflowIr
        ? JSON.stringify({ workflowIr: attachment.workflowIr })
        : undefined,
      reviewCapabilities,
    };
    return [planned, ...workflows];
  }, [attachment, t, workflows]);

  const begin = useCallback((nextDraft?: RoutineDraft) => {
    setDraft(nextDraft ?? null);
    setPreparationBusy(false);
    setPreparationFailed(false);
  }, []);

  const prepare = useCallback(async () => {
    if (!draft?.workflowAttachment) return;
    setPreparationBusy(true);
    setPreparationFailed(false);
    try {
      const workflowAttachment = await composeRoutineTargetWorkflow(
        draft,
        draft.workflowAttachment,
        {
          projectDescription: t("routines.handoff_project_description"),
          projectName: t("routines.handoff_project_name"),
          workflowDescription: t("routines.handoff_workflow_description"),
          workflowName: t("routines.handoff_workflow_name"),
        },
      );
      setDraft((current) =>
        current?.id === draft.id
          ? { ...current, workflowAttachment }
          : current,
      );
    } catch {
      setPreparationFailed(true);
    } finally {
      setPreparationBusy(false);
    }
  }, [draft, t]);

  useEffect(() => {
    if (!preparationRequired || preparationBusy || preparationFailed) return;
    const timer = window.setTimeout(() => void prepare(), 0);
    return () => window.clearTimeout(timer);
  }, [prepare, preparationBusy, preparationFailed, preparationRequired]);

  return {
    attachment,
    begin,
    draft,
    preparationBusy,
    preparationFailed,
    preparationRequired,
    prepare,
    reviewProjects,
    reviewWorkflows,
  };
}
