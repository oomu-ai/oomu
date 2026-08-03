"use client";

import { useState } from "react";
import { useI18n } from "@/context/I18nContext";
import type { P0EventEnvelope } from "@/lib/p0Contracts";
import { useAppShell } from "@/components/AppShell";
import type { TaskRun } from "../../tasks/taskClient";
import {
  documentCreationApi,
  type ContextDocumentKind,
} from "./documentCreationClient";
import { requestDocumentFocus } from "./documentFocus";
import { presentationApi, type RegisteredPresentationTemplate } from "../presentations/presentationClient";

export function CreateDocumentAction({
  events,
  task,
}: {
  events: P0EventEnvelope[];
  task: TaskRun;
}) {
  const { language, t } = useI18n();
  const { setActiveItem } = useAppShell();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<ContextDocumentKind | "">("");
  const [result, setResult] = useState<"created" | "failed" | "">("");
  const [createdId, setCreatedId] = useState("");
  const [createdKind, setCreatedKind] = useState<ContextDocumentKind | "">("");
  const [presentationTemplate, setPresentationTemplate] = useState<RegisteredPresentationTemplate | null>(null);
  const [templateBusy, setTemplateBusy] = useState(false);
  const [templateError, setTemplateError] = useState<"incompatible" | "failed" | "">("");
  const recordedDocument = createdDocumentFromEvents(events);
  const readyId = createdId || recordedDocument?.artifactId || "";
  const readyKind = createdKind || recordedDocument?.kind || "";

  async function create(kind: ContextDocumentKind) {
    setBusy(kind); setResult("");
    const title = (task.summary || t("tasks.untitled")).trim().slice(0, 240);
    const activity = events.map((event) => t("documents.activity_line", {
      kind: evidenceLabel(t, event.evidenceClass),
      time: new Date(event.timestamp).toLocaleString(language),
    }));
    const visibleOutput = events.map((event) => safeVisibleOutput(event.payload)).filter(Boolean);
    const sources = events
      .filter((event) => event.projectId === task.projectId && event.taskId === task.taskId && event.taskRunId === task.taskRunId)
      .slice(0, 32)
      .map((event) => ({
        sourceRef: event.eventType,
        evidenceRef: `task-event:${task.taskRunId}:${event.sequence}`,
      }));
    try {
      const created = await documentCreationApi.createFromTask(kind, task, {
        title,
        summary: [title, ...(visibleOutput.length ? visibleOutput : activity)].join("\n"),
        locale: language,
        sheet: t("documents.task_sheet"),
        item: t("documents.item"),
        value: t("documents.value"),
        summaryLabel: t("documents.task_summary"),
        createdAt: t("documents.created_at"),
        coverLabel: t("presentations.task_cover_label"),
        findingsTitle: t("presentations.task_findings_title"),
        sources,
      }, kind === "powerpoint" ? presentationTemplate : null);
      setCreatedId(created.artifactId); setCreatedKind(kind); setResult("created"); setOpen(false);
    } catch { setResult("failed"); }
    finally { setBusy(""); }
  }

  async function choosePresentationTemplate() {
    if (!task.projectId) return;
    setTemplateBusy(true); setTemplateError("");
    try {
      const selected = await presentationApi.inspectTemplate(task.projectId, task.taskId, task.taskRunId);
      if (!selected) return;
      if (!selected.taskSummaryCompatible) {
        setPresentationTemplate(null); setTemplateError("incompatible"); return;
      }
      setPresentationTemplate(selected);
    } catch { setTemplateError("failed"); }
    finally { setTemplateBusy(false); }
  }

  const hasResult = Boolean(task.summary.trim()) || events.some((event) => Boolean(safeVisibleOutput(event.payload)));
  if (!task.projectId || task.state !== "completed" || !hasResult) return null;

  return (
    <div className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-4">
      {readyId && readyKind ? <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded bg-[var(--success-background)] p-3"><div><p className="text-sm font-semibold">{t("document_labels.ready_from_task")}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("documents.created")}</p></div><button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)]" onClick={() => { requestDocumentFocus(readyKind === "excel" ? "spreadsheet" : readyKind === "powerpoint" ? "presentation" : "word", readyId); setActiveItem("artifacts"); }} type="button">{t("documents.open")}</button></div> : null}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div><h3 className="text-sm font-semibold">{t(readyId ? "document_labels.create_another" : "documents.create_from_task")}</h3><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t(readyId ? "document_labels.create_another_help" : "documents.create_from_task_help")}</p></div>
        <button aria-expanded={open} className="rounded bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)]" onClick={() => setOpen((value) => !value)} type="button">{t(readyId ? "document_labels.create_another" : "documents.create")}</button>
      </div>
      {open ? <div className="mt-3"><div className="grid gap-2 sm:grid-cols-3"><button className="rounded border p-3 text-left text-sm disabled:opacity-50" disabled={Boolean(busy)} onClick={() => void create("word_pdf")} type="button"><span className="block font-semibold">{t("documents.word_pdf")}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{t("documents.word_pdf_help")}</span></button><button className="rounded border p-3 text-left text-sm disabled:opacity-50" disabled={Boolean(busy)} onClick={() => void create("excel")} type="button"><span className="block font-semibold">{t("documents.excel")}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{t("documents.excel_help")}</span></button><button className="rounded border p-3 text-left text-sm disabled:opacity-50" disabled={Boolean(busy)} onClick={() => void create("powerpoint")} type="button"><span className="block font-semibold">{t("documents.powerpoint")}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{t("documents.powerpoint_help")}</span></button></div><div className="mt-2 flex flex-wrap items-center gap-2"><button className="rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={Boolean(busy) || templateBusy} onClick={() => void choosePresentationTemplate()} type="button">{t(presentationTemplate ? "presentation_template.choose_another" : "presentation_template.choose")}</button>{presentationTemplate ? <span className="text-xs text-[var(--foreground-muted)]">{t("presentation_template.selected", { name: presentationTemplate.name })}</span> : null}</div>{templateError ? <p className="mt-2 text-xs text-[var(--warning)]" role="status">{t(`presentation_template.${templateError}`)}</p> : null}</div> : null}
      {result === "failed" ? <p className="mt-3 text-sm text-[var(--warning)]" role="status">{t("documents.create_failed")}</p> : null}
    </div>
  );
}

function createdDocumentFromEvents(events: P0EventEnvelope[]): { artifactId: string; kind: ContextDocumentKind } | null {
  for (const event of [...events].reverse()) {
    const kind = event.eventType === "workbook.review_ready" ? "excel" : event.eventType === "presentation.review_ready" ? "powerpoint" : event.eventType === "artifact.verified" ? "word_pdf" : null;
    if (!kind || !event.payload || typeof event.payload !== "object" || Array.isArray(event.payload)) continue;
    const artifactId = (event.payload as Record<string, unknown>).artifactId;
    if (typeof artifactId === "string" && /^artifact_[0-9a-f-]{36}$/i.test(artifactId)) return { artifactId, kind };
  }
  return null;
}

const KNOWN_EVIDENCE = new Set(["model_assertion", "observed_result", "executed_mutation", "verified_postcondition", "signed_artifact"]);
function evidenceLabel(t: (key: string) => string, value: string) {
  return t(`evidence.classes.${KNOWN_EVIDENCE.has(value) ? value : "unknown"}.label`);
}

function safeVisibleOutput(payload: unknown) {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return "";
  const record = payload as Record<string, unknown>;
  for (const key of ["userVisibleOutput", "visibleOutput", "summary"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim().slice(0, 2_000);
  }
  return "";
}
