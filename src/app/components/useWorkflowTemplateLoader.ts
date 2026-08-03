"use client";

import { useCallback, useEffect, useRef } from "react";
import {
  bindWorkflowSourceFolder,
  instantiateWorkflowIrTemplate,
  localizeWorkflowIrTemplate,
  workflowTemplateById,
  type WorkflowTemplateId,
} from "./workflowLibrary";
import {
  chooseWorkflowSourceFolder,
  type WorkflowSourceFolder,
} from "./workflowSourceFolder";
import type { WorkflowIr } from "./workflowIr";
import type {
  CompiledInstruction,
  SavedWorkflow,
} from "./workflowPersistence";
import { workflowTemplateName } from "./WorkflowComposerScaffolding";
import { recordWorkflowAuthoringMetric } from "./workflowQualityMetrics";

type Translate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

type TemplateDraft = {
  prompt: string;
  source: "template";
  workflow: SavedWorkflow;
  workflowIr: WorkflowIr;
};

type TemplateNotice = {
  message: string;
  tone: "success" | "warning" | "error";
};

type WorkflowTemplateLoaderOptions = {
  createSavedWorkflow: (workflowIr: WorkflowIr, prompt: string) => SavedWorkflow;
  createWorkflowId: () => string;
  formatError: (error: unknown) => string;
  onRequestedTemplateLoaded?: (templateId: WorkflowTemplateId) => void;
  requestedTemplateId: WorkflowTemplateId | null;
  requestedTemplateSourceFolder?: WorkflowSourceFolder | null;
  setCompiledInstructions: (instructions: CompiledInstruction[]) => void;
  setDraft: (draft: TemplateDraft) => void;
  setEditCountBeforeFirstRun: (count: number) => void;
  setInstructionDrafts: (drafts: Record<string, string>) => void;
  setNotice: (notice: TemplateNotice) => void;
  setPrompt: (prompt: string) => void;
  setWorkflowDraft: (draft: null) => void;
  setWorkflowRunStatus: (status: string) => void;
  t: Translate;
};

export function useWorkflowTemplateLoader({
  createSavedWorkflow,
  createWorkflowId,
  formatError,
  onRequestedTemplateLoaded,
  requestedTemplateId,
  requestedTemplateSourceFolder,
  setCompiledInstructions,
  setDraft,
  setEditCountBeforeFirstRun,
  setInstructionDrafts,
  setNotice,
  setPrompt,
  setWorkflowDraft,
  setWorkflowRunStatus,
  t,
}: WorkflowTemplateLoaderOptions) {
  const loadedRequestedTemplateId = useRef<WorkflowTemplateId | null>(null);

  const loadTemplateById = useCallback(
    async (
      templateId: WorkflowTemplateId,
      preparedSourceFolder?: WorkflowSourceFolder | null,
    ) => {
      const template = workflowTemplateById(templateId);
      if (!template) {
        return false;
      }

      try {
        const sourceFolder =
          templateId === "directory-summarizer"
            ? preparedSourceFolder ??
              (await chooseWorkflowSourceFolder({
                title: t(
                  "workflows.templates.directory-summarizer.picker_title",
                ),
                truncationNotice: t(
                  "workflows.templates.directory-summarizer.truncation_file_notice",
                ),
              }))
            : null;
        if (templateId === "directory-summarizer" && !sourceFolder) {
          return false;
        }
        const localizedTemplate = localizeWorkflowIrTemplate(template, t, {
          sourceTruncated: sourceFolder?.truncated,
        });
        let workflowIr = instantiateWorkflowIrTemplate(
          localizedTemplate,
          createWorkflowId(),
        );
        if (sourceFolder) {
          workflowIr = bindWorkflowSourceFolder(workflowIr, sourceFolder.folderPath);
          workflowIr.metadata = {
            ...workflowIr.metadata,
            sourceFolder: {
              fileCount: sourceFolder.fileCount,
              folderName: sourceFolder.folderName,
              totalBytes: sourceFolder.totalBytes,
              truncated: sourceFolder.truncated,
            },
          };
        }
        const workflow = createSavedWorkflow(
          workflowIr,
          localizedTemplate.seedPrompt,
        );
        const templateName = workflowTemplateName(localizedTemplate, t);
        setDraft({
          prompt: localizedTemplate.seedPrompt,
          source: "template",
          workflow,
          workflowIr,
        });
        setPrompt(localizedTemplate.seedPrompt);
        setCompiledInstructions([]);
        setInstructionDrafts({});
        setNotice({
          message: sourceFolder?.truncated
            ? t("workflows.composer.template_loaded_truncated", {
                name: templateName,
              })
            : t("workflows.composer.template_loaded", { name: templateName }),
          tone: sourceFolder?.truncated ? "warning" : "success",
        });
        setEditCountBeforeFirstRun(0);
        recordWorkflowAuthoringMetric("template_loaded");
        setWorkflowRunStatus(t("workflows.composer.review_status"));
        return true;
      } catch (error) {
        setNotice({
          message:
            templateId === "directory-summarizer"
              ? t("workflows.composer.folder_prepare_error")
              : t("workflows.composer.template_error", {
                  error: formatError(error),
                }),
          tone: "error",
        });
        return false;
      }
    },
    [
      createSavedWorkflow,
      createWorkflowId,
      formatError,
      setCompiledInstructions,
      setDraft,
      setEditCountBeforeFirstRun,
      setInstructionDrafts,
      setNotice,
      setPrompt,
      setWorkflowRunStatus,
      t,
    ],
  );

  useEffect(() => {
    if (!requestedTemplateId) {
      loadedRequestedTemplateId.current = null;
      return;
    }
    if (loadedRequestedTemplateId.current === requestedTemplateId) {
      return;
    }

    let cancelled = false;
    queueMicrotask(() => {
      void loadTemplateById(
        requestedTemplateId,
        requestedTemplateSourceFolder,
      ).then((loaded) => {
        if (!cancelled && loaded) {
          loadedRequestedTemplateId.current = requestedTemplateId;
          setWorkflowDraft(null);
          onRequestedTemplateLoaded?.(requestedTemplateId);
        }
      });
    });
    return () => {
      cancelled = true;
    };
  }, [
    loadTemplateById,
    onRequestedTemplateLoaded,
    requestedTemplateId,
    requestedTemplateSourceFolder,
    setWorkflowDraft,
  ]);

  return loadTemplateById;
}
