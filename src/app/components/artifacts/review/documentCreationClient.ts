import { createSimpleArtifactDocument } from "@/lib/artifacts/schema";
import { createTaskSummaryWorkbook } from "@/lib/artifacts/workbooks/schema";
import { workbookIrSchema } from "@/lib/artifacts/workbooks/schema";
import { createTaskSummaryPresentation } from "@/lib/artifacts/presentations/schema";
import type { TaskRun } from "../../tasks/taskClient";
import { artifactApi } from "../artifactClient";
import { workbookApi } from "../workbooks/workbookClient";
import { presentationApi, type RegisteredPresentationTemplate } from "../presentations/presentationClient";

export type ContextDocumentKind = "word_pdf" | "excel" | "powerpoint";

type ContextDocumentCopy = {
  title: string;
  summary: string;
  locale: string;
  sheet: string;
  item: string;
  value: string;
  summaryLabel: string;
  createdAt: string;
  coverLabel: string;
  findingsTitle: string;
  sources: Array<{ sourceRef: string; evidenceRef: string }>;
};

export const documentCreationApi = {
  createWorkbookFromAgentWork: async (task: TaskRun, workbook: unknown) => {
    if (!task.projectId) throw new Error("document_project_required");
    const validated = workbookIrSchema.parse(workbook);
    return workbookApi.create(task.projectId, task.taskId, task.taskRunId, validated);
  },
  createFromTask: async (
    kind: ContextDocumentKind,
    task: TaskRun,
    copy: ContextDocumentCopy,
    presentationTemplate?: RegisteredPresentationTemplate | null,
  ) => {
    if (!task.projectId) throw new Error("document_project_required");
    if (kind === "word_pdf") {
      return artifactApi.create(
        task.projectId,
        task.taskRunId,
        createSimpleArtifactDocument(copy.title, copy.summary, {
          language: copy.locale,
          sources: copy.sources,
        }),
      );
    }
    if (kind === "powerpoint") {
      const presentation = createTaskSummaryPresentation({
        title: copy.title,
        summary: copy.summary,
        locale: copy.locale,
        coverLabel: copy.coverLabel,
        findingsTitle: copy.findingsTitle,
        sources: copy.sources,
      });
      if (presentationTemplate) {
        if (!presentationTemplate.taskSummaryCompatible) throw new Error("presentation_template_incompatible");
        presentation.template = {
          templateId: presentationTemplate.templateId,
          name: presentationTemplate.name,
          imported: true,
          fingerprintSha256: presentationTemplate.fingerprintSha256,
        };
      }
      const created = await presentationApi.create(task.projectId, task.taskId, task.taskRunId, copy.title, presentation);
      return { artifactId: created.summary.presentationId };
    }
    const workbook = createTaskSummaryWorkbook({
      title: copy.title,
      locale: copy.locale,
      summary: copy.summary,
      createdAtIso: new Date(task.createdAtMs).toISOString(),
      labels: {
        sheet: copy.sheet,
        item: copy.item,
        value: copy.value,
        summary: copy.summaryLabel,
        createdAt: copy.createdAt,
      },
      sources: copy.sources,
    });
    return workbookApi.create(task.projectId, task.taskId, task.taskRunId, workbook);
  },
};
