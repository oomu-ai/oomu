"use client";

import Image from "next/image";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { applyRevisionInstruction } from "@/lib/artifacts/revision";
import { projectApi, type ProjectRecord } from "../projects/projectClient";
import {
  artifactApi,
  type ArtifactRecord,
  type ArtifactVersion,
} from "./artifactClient";
import { DocumentReviewShell } from "./review/DocumentReviewShell";
import { consumeDocumentFocus } from "./review/documentFocus";
import { WorkbookDocumentReview } from "./workbooks/WorkbookDocumentReview";
import { workbookApi, type WorkbookReview } from "./workbooks/workbookClient";
import { PresentationDocumentReview } from "./presentations/PresentationDocumentReview";
import { presentationApi, type PresentationReviewSummary } from "./presentations/presentationClient";
import { ScreenEmptyState } from "../shared/ScreenEmptyState";

type LibraryItem =
  | { id: string; kind: "word"; projectId: string; record: ArtifactRecord }
  | { id: string; kind: "spreadsheet"; projectId: string; record: WorkbookReview }
  | { id: string; kind: "presentation"; projectId: string; record: PresentationReviewSummary };

export function ArtifactStudio({
  onOpenSettings = () => {},
  onStartInChat,
}: {
  onOpenSettings?: () => void;
  onStartInChat?: () => void;
}) {
  const { t } = useI18n();
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [projectId, setProjectId] = useState("");
  const [documents, setDocuments] = useState<ArtifactRecord[]>([]);
  const [workbooks, setWorkbooks] = useState<WorkbookReview[]>([]);
  const [presentations, setPresentations] = useState<PresentationReviewSummary[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const items = useMemo<LibraryItem[]>(() => [
    ...documents.map((record) => ({ id: `word:${record.artifactId}`, kind: "word" as const, projectId: record.projectId, record })),
    ...workbooks.map((record) => ({ id: `spreadsheet:${record.artifactId}`, kind: "spreadsheet" as const, projectId: record.projectId, record })),
    ...presentations.map((record) => ({ id: `presentation:${record.presentationId}`, kind: "presentation" as const, projectId: record.projectId, record })),
  ].sort((left, right) => right.record.title.localeCompare(left.record.title)), [documents, presentations, workbooks]);
  const selected = items.find((item) => item.id === selectedId) ?? items[0] ?? null;

  const load = useCallback(async () => {
    setLoading(true);
    const [documentResult, workbookResult, presentationResult] = await Promise.allSettled([
      artifactApi.list(projectId || undefined),
      workbookApi.list(projectId || undefined),
      presentationApi.list(projectId || undefined),
    ]);
    setDocuments(documentResult.status === "fulfilled" ? documentResult.value : []);
    setWorkbooks(workbookResult.status === "fulfilled" ? workbookResult.value : []);
    setPresentations(presentationResult.status === "fulfilled" ? presentationResult.value : []);
    const failures = [documentResult, workbookResult, presentationResult].filter((result) => result.status === "rejected").length;
    setError(failures === 0 ? "" : failures === 3 ? "unavailable" : "partially_unavailable");
    setLoading(false);
  }, [projectId]);

  useEffect(() => { void projectApi.list().then(setProjects).catch(() => setProjects([])); }, []);
  useEffect(() => {
    const timer = window.setTimeout(() => {
      const requested = consumeDocumentFocus();
      if (requested) setSelectedId(requested);
    }, 0);
    return () => window.clearTimeout(timer);
  }, []);
  useEffect(() => {
    const timer = window.setTimeout(() => { void load(); }, 0);
    return () => window.clearTimeout(timer);
  }, [load]);

  return (
    <section className="flex h-full min-h-0 flex-col p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="max-w-3xl text-sm leading-6 text-[var(--foreground-muted)]">{t("documents.subtitle")}</p>
        <button className="rounded border px-3 py-2 text-xs font-semibold" onClick={() => void load()} type="button">{t("common.refresh")}</button>
      </div>
      {error ? <p className="mt-3 text-sm text-[var(--warning)]" role="alert">{t(`documents.${error}`)}</p> : null}
      <div className="mt-5 grid min-h-0 flex-1 grid-cols-[19rem_minmax(0,1fr)] overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)]">
        <aside className="min-h-0 overflow-y-auto border-r border-[var(--border-soft)] p-4">
          <select aria-label={t("documents.project_filter")} className="w-full rounded border bg-[var(--background)] p-2 text-sm" onChange={(event) => setProjectId(event.target.value)} value={projectId}>
            <option value="">{t("documents.all_projects")}</option>
            {projects.map((project) => <option key={project.projectId} value={project.projectId}>{project.name}</option>)}
          </select>
          <h2 className="mt-5 text-xs font-semibold uppercase tracking-wide">{t("documents.library")}</h2>
          <div className="mt-2 grid gap-1">
            {loading ? <p className="p-2 text-sm text-[var(--foreground-muted)]">{t("common.loading")}</p> : null}
            {items.map((item) => <button className={`rounded p-2 text-left ${selected?.id === item.id ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"}`} key={item.id} onClick={() => setSelectedId(item.id)} type="button"><span className="block text-sm font-semibold">{item.record.title}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{item.kind === "word" ? t("documents.word_pdf") : item.kind === "spreadsheet" ? t("documents.excel") : t("documents.powerpoint")}</span></button>)}
          </div>
        </aside>
        <main className="min-h-0 overflow-y-auto p-5">
          {!loading && !selected ? (
            <div className="flex h-full items-center justify-center"><ScreenEmptyState actionLabel={onStartInChat ? t("documents.go_to_chat") : undefined} body={t("documents.empty")} className="w-full" icon={<DocumentsEmptyIcon />} onAction={onStartInChat} title={t("documents.empty_title")} /></div>
          ) : null}
          {selected?.kind === "word" ? <WordDocumentReview key={`${selected.id}:${selected.record.currentVersion}`} onRefresh={load} record={selected.record} /> : null}
          {selected?.kind === "spreadsheet" ? <WorkbookDocumentReview key={`${selected.id}:${selected.record.currentRevision}`} onReplace={(next) => setWorkbooks((current) => current.map((item) => item.artifactId === next.artifactId ? next : item))} review={selected.record} /> : null}
          {selected?.kind === "presentation" ? <PresentationDocumentReview key={`${selected.id}:${selected.record.currentRevision}`} onOpenSetup={onOpenSettings} onRefresh={load} summary={selected.record} /> : null}
        </main>
      </div>
    </section>
  );
}

function DocumentsEmptyIcon() {
  return (
    <svg aria-hidden="true" className="h-10 w-10 text-[var(--foreground-muted)]" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.5" viewBox="0 0 24 24">
      <path d="M6 3h9l3 3v15H6z" />
      <path d="M15 3v4h4M9 12h6M9 16h6" />
    </svg>
  );
}

function WordDocumentReview({ onRefresh, record }: { onRefresh: () => Promise<void>; record: ArtifactRecord }) {
  const { t } = useI18n();
  const [activeVersionNumber, setActiveVersionNumber] = useState(record.currentVersion);
  const [previewState, setPreviewState] = useState<{ key: string; pages: string[] }>({ key: "", pages: [] });
  const [instruction, setInstruction] = useState("");
  const [format, setFormat] = useState<"docx" | "pdf" | "both">("both");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const current = record.versions.find((item) => item.version === record.currentVersion) ?? record.versions[0];
  const safePrior = record.versions.find((item) => item.version < record.currentVersion && item.status === "verified" && item.verification.visuallyVerifiedPdf);
  const version = record.versions.find((item) => item.version === activeVersionNumber) ?? current;
  const latest = version?.version === record.currentVersion;

  const previewKey = version ? `${record.artifactId}:${version.version}` : "";
  const preview = previewState.key === previewKey ? previewState.pages : [];
  useEffect(() => {
    let active = true;
    if (!version || version.status !== "verified") return () => { active = false; };
    void Promise.all(Array.from({ length: version.verification.pageCount }, (_, page) => artifactApi.preview(record.artifactId, version.version, page)))
      .then((pages) => { if (active) setPreviewState({ key: previewKey, pages }); })
      .catch(() => { if (active) setPreviewState({ key: previewKey, pages: [] }); });
    return () => { active = false; };
  }, [previewKey, record.artifactId, version]);

  if (!version) return <p className="text-sm text-[var(--foreground-muted)]">{t("documents.review_unavailable")}</p>;

  async function revise() {
    if (!instruction.trim() || !latest) return;
    setBusy(true); setError("");
    try {
      await artifactApi.revise(record.artifactId, record.projectId, record.taskRunId, instruction.trim(), applyRevisionInstruction(version.document, instruction.trim()));
      setInstruction(""); await onRefresh();
    } catch { setError("revision_failed"); } finally { setBusy(false); }
  }

  async function exportFiles() {
    setBusy(true); setError("");
    try {
      const grant = await artifactApi.chooseExport(record.artifactId, version.version);
      if (grant) await artifactApi.export(record.artifactId, version.version, grant.exportGrantId, format);
    } catch { setError("export_failed"); } finally { setBusy(false); }
  }

  const needsRecovery = latest && version.status === "failed" && Boolean(safePrior);
  const actions = needsRecovery ? (
    <button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)]" onClick={() => setActiveVersionNumber(safePrior!.version)} type="button">{t("documents.open_safe_version")}</button>
  ) : <>
    {!latest ? <button className="rounded border px-3 py-2 text-xs font-semibold" onClick={() => setActiveVersionNumber(record.currentVersion)} type="button">{t("documents.back_to_latest")}</button> : null}
    <button className="rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={busy || version.status !== "verified"} onClick={() => void exportFiles()} type="button">{t("documents.export")}</button>
  </>;

  return <DocumentReviewShell actions={actions} details={<div className="grid gap-5"><div><label className="text-sm font-semibold" htmlFor="document-file-type">{t("documents.file_type")}</label><select className="mt-2 block rounded border bg-[var(--background)] p-2 text-sm" id="document-file-type" onChange={(event) => setFormat(event.target.value as typeof format)} value={format}><option value="both">{t("documents.word_and_pdf")}</option><option value="docx">{t("documents.word")}</option><option value="pdf">{t("documents.pdf")}</option></select><p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("documents.export_receipt_help")}</p></div><div><h3 className="text-sm font-semibold">{t("documents.checks")}</h3><p className="mt-1 text-sm">{version.verification.visuallyVerifiedPdf ? t("documents.pages_checked") : t("documents.checks_pending")}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("documents.page_count", { count: version.verification.pageCount })}</p></div><div><h3 className="text-sm font-semibold">{t("documents.contents")}</h3><ol className="mt-2 list-inside list-decimal text-sm">{version.document.sections.map((section) => <li key={section.heading}>{section.heading}</li>)}</ol></div><div><h3 className="text-sm font-semibold">{t("documents.versions")}</h3><div className="mt-2 flex flex-wrap gap-2">{record.versions.map((item) => <button className="rounded border px-2 py-1 text-xs disabled:opacity-50" disabled={item.status !== "verified" || item.version === version.version} key={item.version} onClick={() => setActiveVersionNumber(item.version)} type="button">{t("documents.open_version", { version: item.version })}</button>)}</div></div>{latest ? <div><h3 className="text-sm font-semibold">{t("documents.make_change")}</h3><textarea aria-label={t("documents.change_instructions")} className="mt-2 w-full rounded border bg-[var(--background)] p-2 text-sm" onChange={(event) => setInstruction(event.target.value)} placeholder={t("documents.change_help")} value={instruction} /><button className="mt-2 rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={busy || !instruction.trim() || version.status !== "verified"} onClick={() => void revise()} type="button">{t("documents.save_new_version")}</button></div> : <p className="text-sm text-[var(--foreground-muted)]">{t("documents.latest_to_edit")}</p>}{error ? <p className="text-sm text-[var(--warning)]" role="alert">{t(`documents.errors.${error}`)}</p> : null}</div>} kind={t("documents.word_pdf")} preview={<PreviewPages pages={preview} version={version} />} revision={version.version} status={<p className="font-medium">{documentStatus(t, version)}</p>} title={record.title} />;
}

function PreviewPages({ pages, version }: { pages: string[]; version: ArtifactVersion }) {
  const { t } = useI18n();
  if (!pages.length) return <p className="text-sm text-[var(--foreground-muted)]">{version.status === "verified" ? t("documents.preview_unavailable") : t("documents.preview_pending")}</p>;
  return <div className="space-y-5">{pages.map((source, index) => <figure className="overflow-hidden rounded border bg-white p-2" key={source}><Image alt={t("documents.preview_alt", { page: index + 1 })} className="h-auto w-full" height={1584} src={source} unoptimized width={1224} /><figcaption className="mt-2 text-center text-xs text-[var(--foreground-muted)]">{t("documents.page", { page: index + 1 })}</figcaption></figure>)}</div>;
}

function documentStatus(t: (key: string) => string, version: ArtifactVersion) {
  if (version.status === "verified" && version.verification.visuallyVerifiedPdf) return t("documents.ready");
  if (version.status === "failed") return t("documents.needs_attention");
  if (version.status === "building") return t("documents.preparing");
  return t("documents.status_unavailable");
}
