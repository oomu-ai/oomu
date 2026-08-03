"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useAppShell } from "@/components/AppShell";
import { McpConfirmationModal } from "@/components/mcp/McpConfirmationModal";
import { useI18n } from "@/context/I18nContext";
import {
  composeWorkflowFromNaturalLanguage,
  editWorkflowFromNaturalLanguage,
  loadWorkflowCapabilityCatalog,
  localizeCapabilityCatalog,
  type CapabilityCatalog,
  type ComposeWorkflowResponse,
} from "./workflowCapabilityCatalog";
import {
  loadCompiledInstructions,
  overrideCompiledInstruction,
  persistWorkflowIr,
  type CompiledInstruction,
  type SavedWorkflow,
  type SaveWorkflowResponse,
  type WorkflowPreflightMode,
} from "./workflowPersistence";
import {
  capabilityMatchesAction,
  compilerErrorCode,
  friendlyAuthoringError,
  localizeMissingCapabilities,
  localizeMissingCapabilityDetails,
  TOPOLOGY_MISSING_REPORT_WRITER_CODE,
  type NoticeCapability,
} from "./workflowComposerNotice";
import {
  workflowTemplates,
  type WorkflowTemplateId,
} from "./workflowLibrary";
import {
  workflowIrSchema,
  type WorkflowIr,
} from "./workflowIr";
import { TrustSummary } from "./TrustSummary";
import { WorkflowStoryboard } from "./WorkflowStoryboard";
import {
  workflowTemplateDescription,
  workflowTemplateName,
} from "./WorkflowComposerScaffolding";
import { useWorkflowRun } from "./useWorkflowRun";
import { useWorkflowTemplateLoader } from "./useWorkflowTemplateLoader";
import type { WorkflowSourceFolder } from "./workflowSourceFolder";
import { recordWorkflowAuthoringMetric } from "./workflowQualityMetrics";
import { insertMissingReportWriter } from "./workflowTopologyRepair";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { RunReport } from "./WorkflowRunReport";
import { WorkflowRunFeedback } from "./WorkflowRunFeedback";
import { WorkflowProjectScopeCard } from "./WorkflowProjectScopeCard";
import {
  InspectDrawer,
  NoticePanel,
  RunProgressPanel,
  SpinnerIcon,
  type ComposerNotice,
} from "./WorkflowComposerPanels";

type ComposerDraft = {
  prompt: string;
  saveResponse?: SaveWorkflowResponse;
  savedWorkflow?: SavedWorkflow;
  source: "describe" | "template" | "saved";
  workflow: SavedWorkflow;
  workflowIr: WorkflowIr;
};

export type WorkflowComposerProps = {
  onRequestedTemplateLoaded?: (templateId: WorkflowTemplateId) => void;
  requestedTemplateId?: WorkflowTemplateId | null;
  requestedTemplateSourceFolder?: WorkflowSourceFolder | null;
};

export function WorkflowComposer({
  onRequestedTemplateLoaded,
  requestedTemplateId = null,
  requestedTemplateSourceFolder = null,
}: WorkflowComposerProps = {}) {
  const { t } = useI18n();
  const {
    setActiveItem,
    setWorkflowDraft,
    setWorkflowsView,
    workflowDraft,
    workflowProjectScope = null,
  } = useAppShell();
  const [workflowName, setWorkflowName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [editInstruction, setEditInstruction] = useState("");
  const [catalog, setCatalog] = useState<CapabilityCatalog | null>(null);
  const [isComposing, setIsComposing] = useState(false);
  const [isEditingWithOomu, setIsEditingWithOomu] = useState(false);
  const [editingWorkflowId, setEditingWorkflowId] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [draft, setDraft] = useState<ComposerDraft | null>(null);
  const [notice, setNotice] = useState<ComposerNotice | null>(null);
  const [isInspectOpen, setIsInspectOpen] = useState(false);
  const [preflightMode, setPreflightMode] = useState<WorkflowPreflightMode>("skipped");
  const [compiledInstructions, setCompiledInstructions] = useState<CompiledInstruction[]>([]);
  const [instructionDrafts, setInstructionDrafts] = useState<Record<string, string>>({});
  const [savingInstructionId, setSavingInstructionId] = useState<string | null>(null);
  const [editCountBeforeFirstRun, setEditCountBeforeFirstRun] = useState(0);
  const workflowRun = useWorkflowRun({
    initialStatus: t("workflows.composer.ready_status"),
  });

  const examples = useMemo(
    () => [
      t("workflows.composer.examples.calendar"),
      t("workflows.composer.examples.email"),
      t("workflows.composer.examples.files"),
    ],
    [t],
  );
  const localizedCatalog = useMemo(
    () => (catalog ? localizeCapabilityCatalog(catalog, t) : null),
    [catalog, t],
  );
  const formatTemplateError = useCallback(
    (error: unknown) => friendlyAuthoringError(error, t),
    [t],
  );
  const createScopedSavedWorkflow = useCallback(
    (workflowIr: WorkflowIr, sourcePrompt: string) =>
      workflowFromIr(
        workflowIr,
        sourcePrompt,
        workflowProjectScope?.projectId ?? null,
      ),
    [workflowProjectScope?.projectId],
  );
  const loadTemplateById = useWorkflowTemplateLoader({
    createSavedWorkflow: createScopedSavedWorkflow,
    createWorkflowId: newWorkflowId,
    formatError: formatTemplateError,
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
    setWorkflowRunStatus: workflowRun.setStatus,
    t,
  });
  const refreshCompiledInstructions = useCallback(
    async (workflowId: string, workflowVersion?: number) => {
      try {
        const instructions = await loadCompiledInstructions(workflowId, workflowVersion);
        setCompiledInstructions(instructions);
        setInstructionDrafts(
          Object.fromEntries(
            instructions.map((instruction) => [
              instruction.id,
              instruction.systemPrompt,
            ]),
          ),
        );
      } catch (error) {
        setNotice({
          message: t("workflows.composer.inspect_load_error", {
            error: friendlyAuthoringError(error, t),
          }),
          tone: "warning",
        });
      }
    },
    [t],
  );

  useEffect(() => {
    let cancelled = false;
    loadWorkflowCapabilityCatalog()
      .then((loadedCatalog) => {
        if (!cancelled) {
          setCatalog(loadedCatalog);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setNotice({
            message: t("workflows.composer.catalog_error", {
              error: friendlyAuthoringError(error, t),
            }),
            tone: "warning",
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  useEffect(() => {
    if (requestedTemplateId || !workflowDraft?.workflowIr) {
      return;
    }
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) {
        return;
      }
      const parsed = workflowIrSchema.safeParse(workflowDraft.workflowIr);
      if (!parsed.success) {
        setNotice({
          message: t("workflows.composer.saved_load_error"),
          tone: "error",
        });
        return;
      }
      const workflow: SavedWorkflow = {
        id: workflowDraft.id ?? parsed.data.workflowId,
        name: workflowDraft.name || parsed.data.name,
        description: workflowDraft.description || parsed.data.description,
        projectId:
          workflowDraft.projectId ?? workflowProjectScope?.projectId ?? null,
        isActive: workflowDraft.isActive ?? true,
        lastRunAt: workflowDraft.lastRunAt,
        workflowIr: parsed.data,
        workflowVersion: workflowDraft.workflowVersion ?? parsed.data.workflowVersion,
        compilationStatus: workflowDraft.compilationStatus,
        createdAt: workflowDraft.createdAt ?? Date.now(),
        updatedAt: Date.now(),
      };
      setDraft({
        prompt: workflow.description,
        savedWorkflow: workflow,
        source: "saved",
        workflow,
        workflowIr: parsed.data,
      });
      setPrompt(workflow.description);
      setNotice({
        message: t("workflows.composer.saved_loaded_notice", { name: workflow.name }),
        tone: "success",
      });
      setWorkflowDraft(null);
      void refreshCompiledInstructions(workflow.id, workflow.workflowVersion);
    });
    return () => {
      cancelled = true;
    };
  }, [
    refreshCompiledInstructions,
    requestedTemplateId,
    setWorkflowDraft,
    t,
    workflowDraft,
    workflowProjectScope,
  ]);

  async function describeWorkflow() {
    const trimmedPrompt = prompt.trim();
    if (!trimmedPrompt) {
      setNotice({
        message: t("workflows.composer.empty_prompt"),
        tone: "warning",
      });
      return;
    }

    setIsComposing(true);
    setDraft(null);
    setNotice(null);
    workflowRun.setStatus(t("workflows.composer.composing_status"));

    try {
      const response = await composeWorkflowFromNaturalLanguage({
        catalog: catalog ?? undefined,
        prompt: trimmedPrompt,
        projectId:
          draft?.workflow.projectId ??
          workflowDraft?.projectId ??
          workflowProjectScope?.projectId ??
          null,
        workflowId: newWorkflowId(),
      });
      recordWorkflowAuthoringMetric(
        response.status === "composed" ? "compose_succeeded" : "compose_failed",
      );
      handleComposeResponse(response, trimmedPrompt, "describe");
    } catch (error) {
      recordWorkflowAuthoringMetric("compose_failed");
      setNotice({
        message: t("workflows.composer.compose_error", {
          error: friendlyAuthoringError(error, t),
        }),
        tone: "error",
      });
      workflowRun.setStatus(t("workflows.composer.compose_failed_status"));
    } finally {
      setIsComposing(false);
    }
  }

  async function askOomuToEdit() {
    if (!draft) {
      return;
    }
    const instruction = editInstruction.trim();
    if (!instruction) {
      setNotice({
        message: t("workflows.composer.empty_edit_instruction"),
        tone: "warning",
      });
      return;
    }

    setIsEditingWithOomu(true);
    setNotice(null);
    workflowRun.setStatus(t("workflows.composer.editing_status"));
    try {
      const response = await editWorkflowFromNaturalLanguage({
        catalog: catalog ?? undefined,
        instruction,
        workflowIr: draft.workflowIr,
      });
      recordWorkflowAuthoringMetric(
        response.status === "composed" ? "edit_succeeded" : "edit_failed",
      );
      handleComposeResponse(response, instruction, draft.source);
      if (response.status === "composed") {
        setEditCountBeforeFirstRun((count) =>
          draft.workflow.lastRunAt || workflowRun.lastRun ? count : count + 1,
        );
        setEditInstruction("");
      }
    } catch (error) {
      recordWorkflowAuthoringMetric("edit_failed");
      setNotice({
        message: t("workflows.composer.edit_error", {
          error: friendlyAuthoringError(error, t),
        }),
        tone: "error",
      });
    } finally {
      setIsEditingWithOomu(false);
    }
  }

  function handleComposeResponse(
    response: ComposeWorkflowResponse,
    sourcePrompt: string,
    source: ComposerDraft["source"],
  ) {
    if (response.status === "composed" && response.workflowIr) {
      // Validate against the same schema the save path enforces, so a draft is never
      // shown that cannot be saved. The engine validates IR with its own (looser) Rust
      // schema, so this is the single gate that guarantees "shown ⇒ saveable."
      const parsed = workflowIrSchema.safeParse(response.workflowIr);
      if (!parsed.success) {
        setNotice({
          message: t("workflows.composer.validation_error"),
          tone: "error",
        });
        workflowRun.setStatus(t("workflows.composer.compose_failed_status"));
        return;
      }
      const namedWorkflowIr = {
        ...parsed.data,
        name: (draft?.workflow.name ?? workflowName).trim() || parsed.data.name,
      };
      const workflow = workflowFromIr(
        namedWorkflowIr,
        sourcePrompt,
        draft?.workflow.projectId ?? workflowProjectScope?.projectId ?? null,
      );
      setDraft({
        prompt: sourcePrompt,
        source,
        workflow,
        workflowIr: namedWorkflowIr,
      });
      setCompiledInstructions([]);
      setInstructionDrafts({});
      if (source === "describe") {
        setEditCountBeforeFirstRun(0);
      }
      setNotice({
        message: source === "describe"
            ? t("workflows.composer.composed_notice")
            : t("workflows.composer.edited_notice"),
        tone: "success",
      });
      workflowRun.setStatus(t("workflows.composer.review_status"));
      return;
    }

    const missingCapabilityDetails = localizeMissingCapabilityDetails(
      response.missingCapabilityDetails,
      catalog,
      t,
    );
    setNotice({
      message: response.reason
        ? friendlyAuthoringError(response.reason, t)
        : t("workflows.composer.compose_failed_status"),
      missingCapabilities: localizeMissingCapabilities(
        response.missingCapabilities,
        catalog,
        t,
      ),
      missingCapabilityDetails,
      tone: response.status === "needs_connection" ? "warning" : "error",
    });
    workflowRun.setStatus(
      response.status === "needs_connection"
        ? t("workflows.composer.needs_connection_status")
        : t("workflows.composer.compose_failed_status"),
    );
  }

  async function buildWithoutCapability(capability: NoticeCapability) {
    const trimmedPrompt = prompt.trim();
    if (!trimmedPrompt) {
      return;
    }

    setIsComposing(true);
    setNotice(null);
    workflowRun.setStatus(t("workflows.composer.composing_status"));

    try {
      const baseCatalog = catalog ?? (await loadWorkflowCapabilityCatalog());
      const filteredCatalog: CapabilityCatalog = {
        ...baseCatalog,
        actions: baseCatalog.actions.filter(
          (action) => !capabilityMatchesAction(action, capability),
        ),
      };
      const response = await composeWorkflowFromNaturalLanguage({
        catalog: filteredCatalog,
        prompt: `${trimmedPrompt}\n\nBuild it without the "${capability.title}" step. Skip that capability instead of asking for a connection.`,
        projectId:
          draft?.workflow.projectId ??
          workflowDraft?.projectId ??
          workflowProjectScope?.projectId ??
          null,
        workflowId: newWorkflowId(),
      });
      recordWorkflowAuthoringMetric(
        response.status === "composed" ? "compose_succeeded" : "compose_failed",
      );
      handleComposeResponse(response, trimmedPrompt, "describe");
    } catch (error) {
      recordWorkflowAuthoringMetric("compose_failed");
      setNotice({
        message: t("workflows.composer.compose_error", {
          error: friendlyAuthoringError(error, t),
        }),
        tone: "error",
      });
      workflowRun.setStatus(t("workflows.composer.compose_failed_status"));
    } finally {
      setIsComposing(false);
    }
  }

  function updateWorkflowName(name: string) {
    setWorkflowName(name);
    setDraft((current) => {
      if (!current) {
        return current;
      }
      const workflowIr = { ...current.workflowIr, name };
      return {
        ...current,
        saveResponse: undefined,
        savedWorkflow: undefined,
        workflow: {
          ...current.workflow,
          name,
          workflowIr,
          updatedAt: Date.now(),
        },
        workflowIr,
      };
    });
  }

  function updateDraftIr(workflowIr: WorkflowIr) {
    setDraft((current) => {
      if (!current) {
        return current;
      }
      const namedWorkflowIr = {
        ...workflowIr,
        name: current.workflow.name,
      };
      const workflow = {
        ...current.workflow,
        description: namedWorkflowIr.description,
        workflowIr: namedWorkflowIr,
        workflowVersion: namedWorkflowIr.workflowVersion,
        updatedAt: Date.now(),
      };
      return {
        ...current,
        saveResponse: undefined,
        savedWorkflow: undefined,
        workflow,
        workflowIr: namedWorkflowIr,
      };
    });
  }

  async function insertReportSaveStepAndRetry() {
    if (!draft) {
      setNotice({
        message: t("workflows.composer.topology_autofix_unavailable"),
        tone: "error",
      });
      return;
    }

    const repaired = insertMissingReportWriter(draft.workflowIr);
    if (!repaired) {
      setNotice({
        message: t("workflows.composer.topology_autofix_unavailable"),
        tone: "error",
      });
      return;
    }

    updateDraftIr(repaired);
    setNotice({
      message: t("workflows.composer.topology_autofix_applied"),
      tone: "info",
    });
    await saveDraft(repaired);
  }

  async function saveDraft(workflowIrOverride?: WorkflowIr) {
    if (!draft) {
      return null;
    }

    const name = draft.workflow.name.trim();
    if (!name) {
      setNotice({
        message: t("workflows.composer.name_required"),
        tone: "warning",
      });
      return null;
    }

    setIsSaving(true);
    setNotice(null);
    const updatedAt = Date.now();
    const workflowIr = {
      ...(workflowIrOverride ?? draft.workflowIr),
      name,
    };
    const workflow: SavedWorkflow = {
      ...draft.workflow,
      description: workflowIr.description || draft.workflow.description,
      name,
      updatedAt,
      workflowIr,
      workflowVersion: workflowIr.workflowVersion,
    };

    try {
      const result = await persistWorkflowIr(
        workflow,
        workflowIr,
        composerVisualState(workflow, workflowIr, draft.prompt, draft.source),
      );
      const savedWorkflow = {
        ...workflow,
        compilationStatus: result.compilationStatus,
        workflowVersion: result.workflowVersion,
      };
      setDraft({
        ...draft,
        saveResponse: result,
        savedWorkflow,
        workflow: savedWorkflow,
        workflowIr,
      });
      recordWorkflowAuthoringMetric("save_succeeded");
      setNotice({
        message: t("workflows.composer.saved_notice", { name: savedWorkflow.name }),
        title: t("workflows.composer.notice_saved"),
        tone: "success",
      });
      workflowRun.setStatus(t("workflows.composer.saved_status"));
      await refreshCompiledInstructions(result.workflowId, result.workflowVersion);
      return savedWorkflow;
    } catch (error) {
      recordWorkflowAuthoringMetric("save_failed");
      if (compilerErrorCode(error) === TOPOLOGY_MISSING_REPORT_WRITER_CODE) {
        setNotice({
          action: {
            kind: "insert_report_save_step",
            label: t("workflows.composer.insert_save_step"),
          },
          message: t("workflows.composer.topology_report_missing_writer"),
          tone: "warning",
        });
        workflowRun.setStatus(t("workflows.composer.topology_needs_fix_status"));
        return null;
      }
      setNotice({
        message: t("workflows.composer.save_error", {
          error: friendlyAuthoringError(error, t),
        }),
        tone: "error",
      });
      return null;
    } finally {
      setIsSaving(false);
    }
  }

  async function saveAndRunDraft() {
    if (!draft) {
      return;
    }
    const savedWorkflow = draft.savedWorkflow ?? (await saveDraft());
    if (!savedWorkflow) {
      return;
    }

    const isFirstRun = !savedWorkflow.lastRunAt && !workflowRun.lastRun;
    const response = await workflowRun.runWorkflow(savedWorkflow, {
      input: {
        description: savedWorkflow.description,
        objective: savedWorkflow.name,
      },
      onLastRunAt: (_workflowId, lastRunAt) => {
        setDraft((current) =>
          current
            ? {
                ...current,
                savedWorkflow: current.savedWorkflow
                  ? { ...current.savedWorkflow, lastRunAt }
                  : current.savedWorkflow,
                workflow: { ...current.workflow, lastRunAt },
              }
            : current,
        );
      },
      preflightMode,
      workflowVersion: savedWorkflow.workflowVersion,
    });
    if (response?.instance.status === "Completed") {
      recordWorkflowAuthoringMetric("run_succeeded", {
        editCountBeforeFirstRun,
        isFirstRun,
      });
      if (isFirstRun) {
        setEditCountBeforeFirstRun(0);
      }
    } else {
      recordWorkflowAuthoringMetric("run_failed");
    }
  }

  async function saveInstructionOverride(instruction: CompiledInstruction) {
    const systemPrompt = instructionDrafts[instruction.id]?.trim();
    if (!systemPrompt) {
      setNotice({
        message: t("workflows.composer.empty_prompt_override"),
        tone: "warning",
      });
      return;
    }
    setSavingInstructionId(instruction.id);
    try {
      const updated = await overrideCompiledInstruction(instruction, systemPrompt);
      setCompiledInstructions((current) =>
        current.map((item) => (item.id === updated.id ? updated : item)),
      );
      setNotice({
        message: t("workflows.composer.prompt_override_saved"),
        tone: "success",
      });
    } catch (error) {
      setNotice({
        message: t("workflows.composer.prompt_override_error", {
          error: friendlyAuthoringError(error, t),
        }),
        tone: "error",
      });
    } finally {
      setSavingInstructionId(null);
    }
  }

  const canSubmit = prompt.trim().length > 0 && !isComposing;
  const isRunning = Boolean(workflowRun.runningWorkflowId);
  const isEditingSteps = draft?.workflow.id === editingWorkflowId;
  const activeWorkflowName = draft?.workflow.name ?? workflowName;
  const hasWorkflowName = activeWorkflowName.trim().length > 0;
  const activeProjectId =
    draft?.workflow.projectId ??
    workflowDraft?.projectId ??
    workflowProjectScope?.projectId ??
    null;
  const activeProjectName =
    activeProjectId && workflowProjectScope?.projectId === activeProjectId
      ? workflowProjectScope.projectName
      : null;

  return (
    <section className="flex h-full min-h-0 flex-col bg-[var(--background)] text-[var(--foreground)]">
      <header className="shrink-0 border-b border-[var(--border-strong)] px-5 py-4">
        {/* No screen title — the sidebar already names the section. */}
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <p
            aria-live="polite"
            className="min-w-0 truncate text-xs text-[var(--foreground-subtle)]"
          >
            {!workflowRun.toast &&
            (isComposing || isEditingWithOomu || isRunning || Boolean(draft))
              ? workflowRun.status
              : ""}
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
              onClick={() => setWorkflowsView("saved_workflows")}
              type="button"
            >
              {t("workflows.composer.open_library")}
            </button>
            {isDeveloperBuild && (
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-transparent px-3 py-2 text-sm font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                onClick={() => setIsInspectOpen(true)}
                type="button"
              >
                {t("workflows.composer.inspect")}
              </button>
            )}
          </div>
        </div>
      </header>

      <WorkflowRunFeedback feedback={workflowRun.toast} />

      <div className="min-h-0 flex-1 overflow-hidden">
        <main className="min-h-0 h-full overflow-y-auto px-5 py-5">
          <div className="mx-auto flex max-w-4xl flex-col gap-5">
            <WorkflowProjectScopeCard
              onChooseProject={() => setActiveItem("projects")}
              projectId={activeProjectId}
              projectName={activeProjectName}
              t={t}
            />
            <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
              <div className="mb-4 max-w-xl">
                <label
                  className="text-sm font-semibold text-[var(--foreground)]"
                  htmlFor="workflow-composer-name"
                >
                  {t("workflows.composer.name_label")}
                </label>
                <input
                  aria-describedby="workflow-composer-name-help"
                  aria-invalid={draft && !hasWorkflowName ? true : undefined}
                  aria-required={draft ? true : undefined}
                  autoComplete="off"
                  className="mt-2 min-h-10 w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:border-[var(--accent)]"
                  disabled={
                    isComposing || isEditingWithOomu || isSaving || isRunning
                  }
                  id="workflow-composer-name"
                  onChange={(event) => updateWorkflowName(event.target.value)}
                  placeholder={t("workflows.composer.name_placeholder")}
                  value={activeWorkflowName}
                />
                <p
                  className={`mt-1.5 text-xs ${
                    draft && !hasWorkflowName
                      ? "text-[var(--warning)]"
                      : "text-[var(--foreground-muted)]"
                  }`}
                  id="workflow-composer-name-help"
                >
                  {draft && !hasWorkflowName
                    ? t("workflows.composer.name_required")
                    : t("workflows.composer.name_help")}
                </p>
              </div>
              <label
                className="text-sm font-semibold text-[var(--foreground)]"
                htmlFor="workflow-composer-prompt"
              >
                {t("workflows.composer.prompt_label")}
              </label>
              <textarea
                aria-describedby="workflow-composer-prompt-help"
                className="mt-3 min-h-28 w-full resize-y rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-3 text-sm leading-6 text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:border-[var(--accent)]"
                disabled={isComposing}
                id="workflow-composer-prompt"
                onChange={(event) => setPrompt(event.target.value)}
                placeholder={t("workflows.composer.prompt_placeholder")}
                value={prompt}
              />
              <p className="sr-only" id="workflow-composer-prompt-help">
                {t("workflows.composer.prompt_help")}
              </p>
              <div className="mt-3 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="flex flex-wrap gap-2">
                  {examples.map((example) => (
                    <button
                      className="rounded-full border border-[var(--border-soft)] bg-[var(--background)] px-3 py-1.5 text-xs font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                      key={example}
                      onClick={() => setPrompt(example)}
                      type="button"
                    >
                      {example}
                    </button>
                  ))}
                </div>
                <button
                  className="inline-flex min-h-10 shrink-0 items-center justify-center gap-2 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-5 py-2.5 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={!canSubmit}
                  onClick={() => void describeWorkflow()}
                  type="button"
                >
                  {isComposing ? <SpinnerIcon /> : <SparkIcon />}
                  {isComposing
                    ? t("workflows.composer.composing")
                    : t("workflows.composer.describe_action")}
                </button>
              </div>
            </section>

            {notice && (
              <NoticePanel
                action={notice.action}
                canBuildWithout={prompt.trim().length > 0 && !isComposing}
                missingCapabilities={notice.missingCapabilities}
                missingCapabilityDetails={notice.missingCapabilityDetails}
                message={notice.message}
                onBuildWithoutCapability={(capability) =>
                  void buildWithoutCapability(capability)
                }
                onConnectCapability={() => setActiveItem("mods")}
                onNoticeAction={(action) => {
                  if (action.kind === "insert_report_save_step") {
                    void insertReportSaveStepAndRetry();
                  }
                }}
                title={notice.title}
                tone={notice.tone}
              />
            )}

            {draft ? (
              <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
                <div className="mb-4 border-b border-[var(--border-soft)] pb-4">
                  <div className="min-w-0">
                    <p className="text-[11px] font-semibold uppercase text-[var(--foreground-subtle)]">
                      {t("workflows.composer.review_label")}
                    </p>
                    {hasWorkflowName ? (
                      <h3 className="mt-1 text-base font-semibold text-[var(--foreground)]">
                        {draft.workflow.name}
                      </h3>
                    ) : null}
                    <p className="mt-1 text-sm leading-5 text-[var(--foreground-muted)]">
                      {draft.workflow.description}
                    </p>
                  </div>
                </div>

                {!isEditingSteps ? (
                  <>
                    <TrustSummary workflowIr={draft.workflowIr} />
                    <div className="mt-4 flex flex-wrap items-center gap-2">
                      <button
                        className="inline-flex min-h-10 items-center gap-1.5 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                        disabled={
                          !hasWorkflowName ||
                          isSaving ||
                          isRunning ||
                          Boolean(workflowRun.approvalRequest)
                        }
                        onClick={() => void saveAndRunDraft()}
                        type="button"
                      >
                        {isRunning ? <SpinnerIcon /> : <PlayIcon />}
                        {isRunning
                          ? t("workflows.composer.running")
                          : t("workflows.composer.run")}
                      </button>
                      <button
                        className="min-h-10 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                        onClick={() => setEditingWorkflowId(draft.workflow.id)}
                        type="button"
                      >
                        {t("workflows.composer.edit_steps")}
                      </button>
                      <button
                        className="min-h-10 rounded-[var(--radius-sm)] px-3 py-2 text-sm font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                        disabled={!hasWorkflowName || isSaving || isRunning}
                        onClick={() => void saveDraft()}
                        type="button"
                      >
                        {isSaving
                          ? t("workflows.composer.saving")
                          : t("workflows.composer.save")}
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
                      <button
                        className="min-h-10 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                        onClick={() => setEditingWorkflowId(null)}
                        type="button"
                      >
                        {t("workflows.composer.done_editing")}
                      </button>
                      <div className="flex flex-wrap gap-2">
                        <button
                          className="min-h-10 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                          disabled={!hasWorkflowName || isSaving || isRunning}
                          onClick={() => void saveDraft()}
                          type="button"
                        >
                          {isSaving
                            ? t("workflows.composer.saving")
                            : t("workflows.composer.save")}
                        </button>
                        <button
                          className="inline-flex min-h-10 items-center gap-1.5 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                          disabled={
                            !hasWorkflowName ||
                            isSaving ||
                            isRunning ||
                            Boolean(workflowRun.approvalRequest)
                          }
                          onClick={() => void saveAndRunDraft()}
                          type="button"
                        >
                          {isRunning ? <SpinnerIcon /> : <PlayIcon />}
                          {isRunning
                            ? t("workflows.composer.running")
                            : t("workflows.composer.run")}
                        </button>
                      </div>
                    </div>

                    <div className="mb-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
                      <label
                        className="text-xs font-semibold text-[var(--foreground)]"
                        htmlFor="workflow-edit-instruction"
                      >
                        {t("workflows.composer.edit_instruction_label")}
                      </label>
                      <div className="mt-2 flex flex-col gap-2 md:flex-row">
                        <input
                          className="min-h-10 flex-1 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-sm text-[var(--foreground)] outline-none placeholder:text-[var(--foreground-subtle)] focus:border-[var(--accent)]"
                          disabled={isEditingWithOomu}
                          id="workflow-edit-instruction"
                          onChange={(event) => setEditInstruction(event.target.value)}
                          placeholder={t("workflows.composer.edit_instruction_placeholder")}
                          value={editInstruction}
                        />
                        <button
                          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                          disabled={isEditingWithOomu}
                          onClick={() => void askOomuToEdit()}
                          type="button"
                        >
                          {isEditingWithOomu
                            ? t("workflows.composer.editing")
                            : t("workflows.composer.ask_to_change")}
                        </button>
                      </div>
                    </div>

                    <WorkflowStoryboard
                      catalog={localizedCatalog}
                      editable
                      onWorkflowIrChange={updateDraftIr}
                      workflowIr={draft.workflowIr}
                    />
                  </>
                )}

                {isRunning && (
                  <RunProgressPanel
                    fallbackLabel={t("workflows.composer.running")}
                    nodes={draft.workflowIr.nodes}
                    progress={workflowRun.progress}
                  />
                )}

                {!isRunning && workflowRun.lastRun && (
                  <RunReport
                    completion={workflowRun.lastRun.completion}
                    durationMs={workflowRun.lastRunDurationMs}
                    executionOrder={workflowRun.lastRun.executionOrder}
                    nodePayloads={workflowRun.lastRun.instance.nodePayloads}
                    nodes={draft.workflowIr.nodes}
                    status={workflowRun.lastRun.instance.status}
                  />
                )}
              </section>
            ) : (
              <section>
                <div className="mb-3">
                  <h3 className="text-sm font-semibold text-[var(--foreground)]">
                    {t("workflows.composer.templates_title")}
                  </h3>
                  <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
                    {t("workflows.composer.templates_description")}
                  </p>
                </div>
                <div className="grid gap-3 md:grid-cols-3">
                  {workflowTemplates.map((template) => (
                    <button
                      className="oomu-interactive-card p-4 text-left"
                      key={template.id}
                      onClick={() => void loadTemplateById(template.id)}
                      aria-label={t("workflows.composer.template_aria", {
                        name: workflowTemplateName(template, t),
                      })}
                      type="button"
                    >
                      <span className="text-[11px] font-semibold uppercase text-[var(--foreground-subtle)]">
                        {t("workflows.composer.template_label")}
                      </span>
                      <span className="mt-2 block text-sm font-semibold text-[var(--foreground)]">
                        {workflowTemplateName(template, t)}
                      </span>
                      <span className="mt-2 line-clamp-3 block text-xs leading-5 text-[var(--foreground-muted)]">
                        {workflowTemplateDescription(template, t)}
                      </span>
                      <span className="mt-3 block text-[11px] font-medium text-[var(--foreground-subtle)]">
                        {t("workflows.library.step_many", {
                          count: template.workflowIr.nodes.filter(
                            (node) => node.kind !== "input" && node.kind !== "output",
                          ).length,
                        })}
                      </span>
                    </button>
                  ))}
                </div>
              </section>
            )}
          </div>
        </main>
      </div>

      {isInspectOpen && (
        <InspectDrawer
          compiledInstructions={compiledInstructions}
          instructionDrafts={instructionDrafts}
          lastRun={workflowRun.lastRun}
          onClose={() => setIsInspectOpen(false)}
          onDraftChange={setInstructionDrafts}
          onPreflightModeChange={setPreflightMode}
          onSaveInstruction={(instruction) => void saveInstructionOverride(instruction)}
          onWorkflowIrChange={updateDraftIr}
          preflightMode={preflightMode}
          savingInstructionId={savingInstructionId}
          workflowIr={draft?.workflowIr}
        />
      )}

      <McpConfirmationModal
        argumentsLabel={workflowRun.approvalPreview?.argumentsLabel}
        argumentsValue={workflowRun.approvalPreview?.argumentsValue ?? {}}
        approveLabel={
          workflowRun.isResolvingApproval
            ? t("workflows.library.approving")
            : workflowRun.approvalPreview?.reusableForWorkflowVersion
              ? t("approvals.approve_for_workflow")
              : t("workflows.library.approve")
        }
        canApprove={workflowRun.approvalPreview?.canApprove ?? false}
        isOpen={Boolean(workflowRun.approvalRequest)}
        isResolving={workflowRun.isResolvingApproval}
        onApprove={() => void workflowRun.resolveApproval("approve")}
        onCancel={() => void workflowRun.resolveApproval("reject")}
        serverLabel={workflowRun.approvalPreview?.serverLabel}
        serverName={workflowRun.approvalPreview?.serverName ?? ""}
        scopeNotice={
          workflowRun.approvalPreview?.reusableForWorkflowVersion
            ? t("approvals.reuse_scope_notice")
            : undefined
        }
        title={t("workflows.library.approve_step")}
        toolLabel={workflowRun.approvalPreview?.toolLabel}
        toolName={workflowRun.approvalPreview?.toolName ?? ""}
      />
    </section>
  );
}

function workflowFromIr(
  workflowIr: WorkflowIr,
  prompt: string,
  projectId: string | null = null,
): SavedWorkflow {
  const timestamp = Date.now();
  return {
    id: workflowIr.workflowId,
    name: workflowIr.name,
    description: workflowIr.description || prompt,
    projectId,
    isActive: true,
    workflowIr,
    workflowVersion: workflowIr.workflowVersion,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
}

function composerVisualState(
  workflow: SavedWorkflow,
  workflowIr: WorkflowIr,
  prompt: string,
  source: ComposerDraft["source"],
) {
  return {
    description: workflow.description,
    isActive: workflow.isActive,
    lastRunAt: workflow.lastRunAt,
    prompt,
    projectId: workflow.projectId ?? null,
    source,
    workflowIr,
    workflowVersion: workflow.workflowVersion,
  };
}

function newWorkflowId() {
  const id =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : Math.random().toString(36).slice(2);
  return `wf-${id}`;
}

function SparkIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M12 3 9.7 9.7 3 12l6.7 2.3L12 21l2.3-6.7L21 12l-6.7-2.3L12 3Z" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg aria-hidden="true" className="h-3 w-3" fill="currentColor" viewBox="0 0 24 24">
      <path d="M8 5.5v13l11-6.5-11-6.5Z" />
    </svg>
  );
}
