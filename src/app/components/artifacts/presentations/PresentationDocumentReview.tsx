"use client";

import Image from "next/image";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import type { PresentationIr } from "@/lib/artifacts/presentations/schema";
import { presentationApi, type PresentationReviewDetail, type PresentationReviewIssue, type PresentationReviewSummary } from "./presentationClient";

const STATUS_KEYS = new Set(["building", "check_required", "ready", "failed"]);
const ISSUE_KEYS = new Set(["missing_font", "font_substituted", "empty_placeholder", "text_overflow", "text_shrunk_to_fit", "contrast_failure", "broken_chart", "missing_asset", "low_resolution_image", "image_resolution_low", "image_license_unknown", "element_overlap", "citation_omission", "preview_unavailable", "imported_package_render_unavailable", "animations_removed", "structure_invalid", "template_unqualified"]);
const CHECK_KEYS = new Set(["package_structure_valid", "typed_projection_matches", "imported_template_mapping_matches", "editable_native_objects", "active_content_absent", "all_slides_semantically_rendered"]);
const ISSUE_ALIASES: Record<string, string> = { exact_package_preview_unavailable: "presentation_issues.exact_package_preview_unavailable", semantic_checks_unavailable: "presentation_issues.semantic_checks_unavailable" };
const CHECK_ALIASES: Record<string, string> = { exact_package_pages_rendered: "presentation_checks.exact_package_pages_rendered", semantic_checks_completed: "presentation_checks.semantic_checks_completed" };

export function PresentationDocumentReview({ onOpenSetup = () => {}, onRefresh, summary }: { onOpenSetup?: () => void; onRefresh: () => Promise<void>; summary: PresentationReviewSummary }) {
  const { t } = useI18n();
  const [detail, setDetail] = useState<PresentationReviewDetail | null>(null);
  const [selectedSlideId, setSelectedSlideId] = useState("");
  const [selectedObjectId, setSelectedObjectId] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<"load" | "revise" | "recheck" | "export" | "">("");
  const [editing, setEditing] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, string>>({});

  async function load(revision?: number) {
    setLoading(true); setError("");
    try {
      let next = await presentationApi.get(summary.presentationId, revision);
      try {
        const preview = await presentationApi.preview(summary.presentationId, next.selectedRevision);
        next = { ...next, filmstrip: preview.filmstrip, issues: preview.issues };
      } catch {
        // The review remains useful while a preview is still being prepared.
      }
      setDetail(next);
      setSelectedSlideId((current) => next.filmstrip.some((item) => item.slideId === current) ? current : next.filmstrip[0]?.slideId ?? "");
      setSelectedObjectId("");
      setEditing(false); setDrafts({});
    } catch { setError("load"); }
    finally { setLoading(false); }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => { void load(); }, 0);
    return () => window.clearTimeout(timer);
  }, [summary.presentationId, summary.currentRevision]); // eslint-disable-line react-hooks/exhaustive-deps

  const selectedSlide = detail?.presentation.slides.find((slide) => slide.slideId === selectedSlideId) ?? detail?.presentation.slides[0];
  const selectedFilmstrip = detail?.filmstrip.find((slide) => slide.slideId === selectedSlide?.slideId) ?? detail?.filmstrip[0];
  const selectedElement = selectedSlide?.elements.find((element) => element.objectId === selectedObjectId);
  const textElements = useMemo(() => selectedSlide?.elements.filter((element) => element.content.kind === "text_box" || (element.content.kind === "shape" && element.content.text)) ?? [], [selectedSlide]);
  const selectedIssues = detail?.issues.filter((issue) => !issue.slideId || issue.slideId === selectedSlide?.slideId) ?? [];
  const rendererMissing = detail?.issues.some((issue) => issue.code === "exact_package_preview_unavailable") ?? false;
  const latest = detail?.selectedRevision === detail?.summary.currentRevision;

  function beginEditing() {
    if (!selectedSlide) return;
    setDrafts(Object.fromEntries(textElements.map((element) => [element.objectId, elementText(element)])));
    setEditing(true);
  }

  async function saveSlide() {
    if (!detail || !selectedSlide || !latest) return;
    const changedObjectIds = textElements
      .filter((element) => drafts[element.objectId] !== elementText(element))
      .map((element) => element.objectId);
    if (!changedObjectIds.length) { setEditing(false); return; }
    const presentation = reviseSlideText(detail.presentation, selectedSlide.slideId, drafts);
    setBusy(true); setError("");
    try {
      const elementScoped = changedObjectIds.length === 1;
      const next = await presentationApi.revise(detail.summary.presentationId, detail.summary.currentRevision, elementScoped ? "element" : "narrative_section", [selectedSlide.slideId], t("presentations.change_summary", { slide: selectedSlide.title || selectedFilmstrip?.title || selectedFilmstrip?.position || 1 }), presentation, elementScoped ? changedObjectIds : []);
      setDetail(next); setEditing(false); setDrafts({});
      await onRefresh();
    } catch { setError("revise"); }
    finally { setBusy(false); }
  }

  async function exportDeck() {
    if (!detail?.verification.exportable) return;
    setBusy(true); setError("");
    try {
      const grant = await presentationApi.chooseExport(detail.summary.presentationId, detail.selectedRevision, `${detail.summary.title}.pptx`);
      if (grant) await presentationApi.export(detail.summary.presentationId, detail.selectedRevision, grant.grantToken);
    } catch { setError("export"); }
    finally { setBusy(false); }
  }

  async function recheckDeck() {
    if (!detail || !latest || !rendererMissing) return;
    setBusy(true); setError("");
    try {
      const next = await presentationApi.recheck(
        detail.summary.presentationId,
        detail.summary.currentRevision,
      );
      setDetail(next);
      await onRefresh();
    } catch { setError("recheck"); }
    finally { setBusy(false); }
  }

  if (loading && !detail) return <p className="text-sm text-[var(--foreground-muted)]">{t("common.loading")}</p>;
  if (!detail) return <p className="text-sm text-[var(--warning)]" role="alert">{t("presentations.errors.load")}</p>;

  const status = knownStatus(detail.summary.status);
  return <section className="grid gap-5">
    <header className="flex flex-wrap items-start justify-between gap-3"><div><p className="text-xs font-semibold uppercase tracking-wide text-[var(--foreground-subtle)]">{t("documents.powerpoint")}</p><h2 className="mt-1 text-xl font-semibold">{detail.summary.title}</h2><p className="mt-2 text-sm text-[var(--foreground-muted)]">{t(`presentations.status.${status}`)}</p></div><button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-40" disabled={busy || !detail.verification.exportable} onClick={() => void exportDeck()} type="button">{t("presentations.export")}</button></header>

    {detail.summary.blockerCount > 0 ? <div className="rounded-[var(--radius-sm)] bg-[var(--warning-background)] p-3"><p className="text-sm font-semibold">{t("presentations.needs_attention")}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("presentations.needs_attention_help")}</p></div> : null}
    {rendererMissing && latest ? <div className="rounded-[var(--radius-sm)] border border-[var(--warning)] p-3"><p className="text-sm font-semibold">{t("presentation_renderer.missing")}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("presentation_renderer.next_step")}</p><div className="mt-3 flex flex-wrap gap-2"><button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)]" onClick={onOpenSetup} type="button">{t("presentation_renderer.open_setup")}</button><button className="rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={busy} onClick={() => void recheckDeck()} type="button">{busy ? t("presentation_renderer.checking") : t("presentation_renderer.check_again")}</button></div></div> : null}

    <div className="grid min-h-[28rem] grid-cols-[9rem_minmax(0,1fr)] overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)]">
      <nav aria-label={t("presentations.slides")} className="overflow-y-auto border-r border-[var(--border-soft)] bg-[var(--accent-background)] p-2">
        {detail.filmstrip.map((slide) => <button aria-current={slide.slideId === selectedSlide?.slideId ? "true" : undefined} className={`mb-2 w-full rounded p-2 text-left ${slide.slideId === selectedSlide?.slideId ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"}`} key={slide.slideId} onClick={() => { setSelectedSlideId(slide.slideId); setSelectedObjectId(""); setEditing(false); }} type="button"><span className="block text-[10px] text-[var(--foreground-muted)]">{t("presentations.slide_number", { number: slide.position + 1 })}</span>{slide.thumbnail ? <Image alt={t("presentations.preview_alt", { number: slide.position + 1 })} className="mt-1 h-auto w-full rounded border border-[var(--border-soft)]" height={slide.thumbnail.height} src={`data:${slide.thumbnail.mediaType};base64,${slide.thumbnail.bytesBase64}`} unoptimized width={slide.thumbnail.width} /> : <span className="mt-1 block aspect-video rounded border border-[var(--border-soft)] bg-[var(--background)]" aria-hidden="true" />}<span className="mt-1 block truncate text-xs font-medium">{slide.title || t("presentations.untitled_slide")}</span>{slide.blockerCount ? <span className="mt-1 block text-[10px] font-semibold text-[var(--warning)]">{t("presentations.slide_needs_attention")}</span> : null}</button>)}
      </nav>
      <div className="min-w-0 p-4">
        {selectedFilmstrip?.thumbnail ? <div className="relative mx-auto w-fit max-w-full"><Image alt={t("presentations.preview_alt", { number: selectedFilmstrip.position + 1 })} className="h-auto max-h-[34rem] w-auto max-w-full rounded border border-[var(--border-soft)] shadow-sm" height={selectedFilmstrip.thumbnail.height} src={`data:${selectedFilmstrip.thumbnail.mediaType};base64,${selectedFilmstrip.thumbnail.bytesBase64}`} unoptimized width={selectedFilmstrip.thumbnail.width} />{selectedElement ? <span aria-label={t("presentations.what_needs_attention")} className="pointer-events-none absolute rounded-sm border-2 border-[var(--warning)] bg-[var(--warning-background)]/20" role="img" style={elementFrameStyle(selectedElement.frame, detail.presentation.aspectRatio)} /> : null}</div> : <div className="grid aspect-video w-full place-items-center rounded border border-dashed border-[var(--border-strong)] bg-[var(--accent-background)] p-8 text-center"><div><p className="text-sm font-semibold">{t("presentations.preview_pending")}</p><p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("presentations.preview_pending_help")}</p></div></div>}
      </div>
    </div>

    {selectedIssues.length ? <section><h3 className="text-sm font-semibold">{t("presentations.what_needs_attention")}</h3><ul className="mt-2 grid gap-2">{selectedIssues.map((issue) => <IssueRow issue={issue} key={issue.issueId} onShow={(slideId, objectId) => { setSelectedSlideId(slideId); setSelectedObjectId(objectId ?? ""); }} />)}</ul></section> : <p className="rounded bg-[var(--success-background)] p-3 text-sm font-medium">{t("presentations.slide_checked")}</p>}

    {latest ? <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4"><div className="flex flex-wrap items-center justify-between gap-3"><div><h3 className="text-sm font-semibold">{t("presentations.edit_slide")}</h3><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("presentations.edit_slide_help")}</p></div>{!editing ? <button className="rounded border px-3 py-2 text-sm font-semibold" onClick={beginEditing} type="button">{t("presentations.edit")}</button> : null}</div>{editing ? <div className="mt-4 grid gap-3">{textElements.map((element, index) => <label className="grid gap-1 text-sm" key={element.objectId}><span className="font-medium">{t(index === 0 ? "presentations.text_title" : "presentations.text_body", { number: index })}</span><textarea className="min-h-24 rounded border bg-[var(--background)] p-2" onChange={(event) => setDrafts((current) => ({ ...current, [element.objectId]: event.target.value }))} value={drafts[element.objectId] ?? ""} /></label>)}<div className="flex gap-2"><button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-40" disabled={busy || !textElements.length} onClick={() => void saveSlide()} type="button">{t("presentations.save_version")}</button><button className="rounded border px-3 py-2 text-sm" disabled={busy} onClick={() => setEditing(false)} type="button">{t("common.cancel")}</button></div></div> : null}</section> : <p className="rounded bg-[var(--accent-background)] p-3 text-sm">{t("presentations.prior_version_read_only")}</p>}

    <details className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4"><summary className="cursor-pointer text-sm font-semibold">{t("common.details")}</summary><div className="mt-4 grid gap-5 md:grid-cols-2"><div><h3 className="text-sm font-semibold">{t("presentations.versions")}</h3><div className="mt-2 flex flex-wrap gap-2">{detail.revisionHistory.map((revision) => <button aria-current={revision.revision === detail.selectedRevision ? "true" : undefined} className="rounded border px-2 py-1 text-xs" key={revision.revision} onClick={() => void load(revision.revision)} type="button">{t("documents.revision", { revision: revision.revision })}</button>)}</div></div><div><h3 className="text-sm font-semibold">{t("presentations.template")}</h3><p className="mt-2 text-sm">{detail.templateIdentity.name}</p></div><div><h3 className="text-sm font-semibold">{t("presentations.sources")}</h3>{detail.citations.length ? <ul className="mt-2 grid gap-2 text-sm">{detail.citations.map((citation) => { const source = detail.provenance.find((item) => item.sourceRef === citation.sourceRef && item.evidenceRef === citation.evidenceRef); const slide = detail.filmstrip.find((item) => item.slideId === citation.slideId); return <li key={citation.citationId}><span className="font-medium">{citation.label}</span><span className="block text-xs text-[var(--foreground-muted)]">{t("presentations.source_used_on", { slide: slide?.title || t("presentations.untitled_slide") })}{source ? ` · ${t(`evidence.classes.${knownEvidenceClass(source.evidenceClass)}.label`)}` : ""}</span></li>; })}</ul> : <p className="mt-2 text-sm text-[var(--foreground-muted)]">{t("presentations.no_sources")}</p>}</div><div><h3 className="text-sm font-semibold">{t("presentations.speaker_notes")}</h3><p className="mt-2 whitespace-pre-wrap text-sm text-[var(--foreground-muted)]">{detail.notes.find((item) => item.slideId === selectedSlide?.slideId)?.speakerNotes || t("presentations.no_notes")}</p></div><div className="md:col-span-2"><h3 className="text-sm font-semibold">{t("presentations.checks")}</h3><ul className="mt-2 grid gap-1 text-sm">{detail.verification.checks.map((check) => <li key={`${check.code}:${check.slideId ?? "deck"}:${check.objectId ?? "all"}`}>{check.passed ? t("presentations.check_passed") : t("presentations.check_failed")} · {t(CHECK_KEYS.has(check.code) ? `presentations.check.${check.code}` : CHECK_ALIASES[check.code] ?? "presentations.check.other")}</li>)}</ul></div></div></details>
    {error ? <p className="text-sm text-[var(--warning)]" role="alert">{t(error === "recheck" ? "presentation_renderer.recheck_failed" : `presentations.errors.${error}`)}</p> : null}
  </section>;
}

function IssueRow({ issue, onShow }: { issue: PresentationReviewIssue; onShow: (slideId: string, objectId?: string | null) => void }) {
  const { t } = useI18n();
  return <li className="flex flex-wrap items-center justify-between gap-2 rounded border border-[var(--border-soft)] p-3"><p className="text-sm">{t(ISSUE_KEYS.has(issue.code) ? `presentations.issue.${issue.code}` : ISSUE_ALIASES[issue.code] ?? "presentations.issue.other")}</p>{issue.slideId ? <button className="rounded border px-2 py-1 text-xs font-semibold" onClick={() => onShow(issue.slideId!, issue.objectId)} type="button">{t("presentations.show_slide")}</button> : null}</li>;
}

function elementFrameStyle(frame: PresentationIr["slides"][number]["elements"][number]["frame"], aspectRatio: PresentationIr["aspectRatio"]) {
  const width = aspectRatio === "16:9" ? 12_192_000 : 9_144_000;
  const height = 6_858_000;
  return { left: `${(frame.x / width) * 100}%`, top: `${(frame.y / height) * 100}%`, width: `${(frame.width / width) * 100}%`, height: `${(frame.height / height) * 100}%` };
}

function knownStatus(value: string) { return STATUS_KEYS.has(value) ? value : "check_required"; }

function knownEvidenceClass(value: string) {
  return ["model_assertion", "observed_result", "executed_mutation", "verified_postcondition", "signed_artifact"].includes(value) ? value : "unknown";
}

function elementText(element: PresentationIr["slides"][number]["elements"][number]) {
  const block = element.content.kind === "text_box" ? element.content.text : element.content.kind === "shape" ? element.content.text : null;
  return block?.paragraphs.map((paragraph) => paragraph.runs.map((run) => run.text).join("")).join("\n") ?? "";
}

function reviseSlideText(presentation: PresentationIr, slideId: string, drafts: Record<string, string>): PresentationIr {
  const slides = presentation.slides.map((slide) => {
    if (slide.slideId !== slideId) return slide;
    const elements = slide.elements.map((element) => {
      if (!(element.objectId in drafts)) return element;
      if (drafts[element.objectId] === elementText(element)) return element;
      const content = element.content.kind === "text_box"
        ? { ...element.content, text: replaceBlock(element.content.text, drafts[element.objectId]) }
        : element.content.kind === "shape" && element.content.text
          ? { ...element.content, text: replaceBlock(element.content.text, drafts[element.objectId]) }
          : element.content;
      return { ...element, content };
    });
    const first = elements.find((element) => element.objectId in drafts);
    return { ...slide, title: first ? drafts[first.objectId].split("\n")[0].slice(0, 512) || slide.title : slide.title, elements };
  });
  return { ...presentation, revision: presentation.revision + 1, slides };
}

function replaceBlock(block: Extract<PresentationIr["slides"][number]["elements"][number]["content"], { kind: "text_box" }>["text"], value: string) {
  const fallback = { text: "", fontFamily: "Arial", fontSizePt: 20, bold: false, italic: false, color: "202124" };
  const templates = block.paragraphs.length ? block.paragraphs : [{ runs: [fallback], alignment: "left" as const, bullet: false }];
  return {
    ...block,
    paragraphs: value.split("\n").slice(0, 256).map((line, index) => {
      const template = templates[Math.min(index, templates.length - 1)];
      const sourceRuns = template.runs.length ? template.runs : [fallback];
      const remaining = Array.from(line);
      const runs = sourceRuns.map((run, runIndex) => {
        const originalLength = Array.from(run.text).length;
        const take = runIndex === sourceRuns.length - 1 ? remaining.length : Math.min(originalLength, remaining.length);
        return { ...run, text: remaining.splice(0, take).join("") };
      });
      return { runs, alignment: template.alignment, bullet: template.bullet };
    }),
  };
}
