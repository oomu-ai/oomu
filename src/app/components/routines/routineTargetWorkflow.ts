import { projectApi } from "../projects/projectClient";
import {
  composeWorkflowFromNaturalLanguage,
  loadWorkflowCapabilityCatalog,
} from "../workflowCapabilityCatalog";
import {
  instantiateWorkflowIrTemplate,
  workflowTemplateById,
} from "../workflowLibrary";
import { workflowIrSchema } from "../workflowIr";
import {
  loadSavedWorkflows,
  persistWorkflowIr,
  type SavedWorkflow,
} from "../workflowPersistence";
import type {
  RoutineHandoffRequest,
  RoutineWorkflowAttachment,
} from "./routineDraft";

export type RoutineTargetWorkflowCopy = {
  projectDescription: string;
  projectName: string;
  workflowDescription: string;
  workflowName: string;
};

const UNREAD_MAIL_TEMPLATE_ID = "unread-mail-check" as const;

export function plannedRoutineWorkflowAttachment(
  request: RoutineHandoffRequest,
  draftId: string,
  projectId: string | null,
): RoutineWorkflowAttachment {
  const workflowId = `workflow-chat-schedule-${draftId}`;
  const mailTemplate = request.targetAction?.kind === "read_unread_mail"
    ? workflowTemplateById(UNREAD_MAIL_TEMPLATE_ID)
    : null;
  return {
    projectPlanned: !projectId,
    projectId: projectId || `planned-project-${draftId}`,
    ...(mailTemplate
      ? { workflowIr: instantiateWorkflowIrTemplate(mailTemplate, workflowId) }
      : {}),
    workflowId,
    workflowName: mailTemplate?.name ?? "Scheduled task",
    workflowVersion: 1,
  };
}

export async function composeRoutineTargetWorkflow(
  request: RoutineHandoffRequest,
  attachment: RoutineWorkflowAttachment,
  copy: RoutineTargetWorkflowCopy,
): Promise<RoutineWorkflowAttachment> {
  if (attachment.workflowIr) return attachment;
  const catalog = await loadWorkflowCapabilityCatalog();
  const response = await composeWorkflowFromNaturalLanguage({
    catalog,
    prompt: request.requestText,
    projectId: attachment.projectPlanned ? null : attachment.projectId,
    workflowId: attachment.workflowId,
    name: copy.workflowName,
  });
  if (response.status !== "composed" || !response.workflowIr) {
    throw new Error(response.reason || "routine_target_workflow_compose_failed");
  }
  const parsed = workflowIrSchema.parse({
    ...response.workflowIr,
    workflowId: attachment.workflowId,
    name: response.workflowIr.name || copy.workflowName,
  });
  return {
    ...attachment,
    workflowIr: parsed,
    workflowName: parsed.name,
    workflowVersion: parsed.workflowVersion,
  };
}

function isExactUnreadMailWorkflow(workflow: SavedWorkflow) {
  const mailNodes = workflow.workflowIr.nodes.filter(
    (node) => node.kind === "mcp_tool" && node.toolName === "read_system_emails",
  );
  return (
    workflow.compilationStatus === "Compiled" &&
    mailNodes.length === 1 &&
    mailNodes[0].kind === "mcp_tool" &&
    mailNodes[0].serverName === "macos_applescript" &&
    JSON.stringify(mailNodes[0].arguments) ===
      JSON.stringify({ max_messages: 20, unread_only: true }) &&
    !workflow.workflowIr.nodes.some(
      (node) =>
        node.kind === "mcp_tool" &&
        (node.toolName === "add_system_reminder" ||
          node.toolName === "draft_system_email" ||
          node.toolName === "send_system_email"),
    )
  );
}

async function resolveProjectId(
  attachment: RoutineWorkflowAttachment,
  copy: RoutineTargetWorkflowCopy,
) {
  if (!attachment.projectPlanned) return attachment.projectId;
  const projects = await projectApi.list();
  const existing = projects.find(
    (project) =>
      project.name === copy.projectName &&
      project.description === copy.projectDescription &&
      project.dataPolicy === "local_only",
  );
  if (existing) return existing.projectId;
  const created = await projectApi.create(
    copy.projectName,
    copy.projectDescription,
    "local_only",
  );
  return created.projectId;
}

export async function materializeRoutineTargetWorkflow(
  attachment: RoutineWorkflowAttachment,
  copy: RoutineTargetWorkflowCopy,
): Promise<RoutineWorkflowAttachment> {
  if (!attachment.workflowIr) {
    throw new Error("routine_target_workflow_not_composed");
  }
  const projectId = await resolveProjectId(attachment, copy);
  const existing = (await loadSavedWorkflows()).find(
    (workflow) =>
      workflow.id === attachment.workflowId && workflow.projectId === projectId,
  );
  if (existing) {
    if (
      existing.compilationStatus !== "Compiled" ||
      existing.reviewCapabilities?.status !== "ready" ||
      (attachment.workflowIr.metadata?.templateId === UNREAD_MAIL_TEMPLATE_ID &&
        !isExactUnreadMailWorkflow(existing))
    ) {
      throw new Error("routine_target_workflow_contract_mismatch");
    }
    return {
      ...attachment,
      projectId,
      projectPlanned: false,
      workflowName: existing.name,
      workflowVersion: existing.workflowVersion ?? 1,
    };
  }

  const now = Date.now();
  const workflowIr = {
    ...attachment.workflowIr,
    name: copy.workflowName,
    description: copy.workflowDescription,
  };
  const workflow: SavedWorkflow = {
    id: attachment.workflowId,
    name: copy.workflowName,
    description: copy.workflowDescription,
    projectId,
    isActive: true,
    workflowIr,
    workflowVersion: 1,
    compilationStatus: "Draft",
    createdAt: now,
    updatedAt: now,
  };
  const saved = await persistWorkflowIr(workflow, workflowIr);
  if (
    saved.compilationStatus !== "Compiled" ||
    saved.projectId !== projectId ||
    saved.reviewCapabilities.status !== "ready" ||
    (attachment.workflowIr.metadata?.templateId === UNREAD_MAIL_TEMPLATE_ID &&
      !saved.reviewCapabilities.emailRead)
  ) {
    throw new Error("routine_target_workflow_unverified");
  }
  return {
    ...attachment,
    projectId,
    projectPlanned: false,
    workflowName: copy.workflowName,
    workflowVersion: saved.workflowVersion,
  };
}
