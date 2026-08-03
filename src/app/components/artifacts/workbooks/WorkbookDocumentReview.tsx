"use client";

import Image from "next/image";
import { useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { DocumentReviewShell } from "../review/DocumentReviewShell";
import {
  canExportWorkbook,
  workbookApi,
  type WorkbookReview,
  type WorkbookSheetReview,
} from "./workbookClient";

export function WorkbookDocumentReview({
  onReplace,
  review,
}: {
  onReplace: (next: WorkbookReview) => void;
  review: WorkbookReview;
}) {
  const { t } = useI18n();
  const [activeRevision, setActiveRevision] = useState(review.currentRevision);
  const [sheetId, setSheetId] = useState(review.sheets[0]?.sheetId ?? "");
  const [previewState, setPreviewState] = useState<{ key: string; source: string | null; unavailable: boolean }>({
    key: "",
    source: null,
    unavailable: false,
  });
  const [cells, setCells] = useState("");
  const [instruction, setInstruction] = useState("");
  const [specificCells, setSpecificCells] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [highlightedCells, setHighlightedCells] = useState<string[]>([]);
  const cellsInput = useRef<HTMLInputElement>(null);
  const activeVersion = review.versionReviews.find((item) => item.revision === activeRevision)
    ?? review.versionReviews.find((item) => item.revision === review.currentRevision);
  const activeReview = activeVersion ? {
    ...review,
    currentRevision: activeVersion.revision,
    calculation: activeVersion.calculation,
    verification: activeVersion.verification,
    sheets: activeVersion.sheets,
    exportable: activeVersion.exportable,
  } : review;
  const latest = activeReview.currentRevision === review.currentRevision;
  const failedCurrent = review.revisions.some((item) => item.revision === review.currentRevision && item.status === "failed");
  const safePrior = review.safePriorRevision && review.revisions.some((item) => item.revision === review.safePriorRevision && item.recoverable) && review.versionReviews.some((item) => item.revision === review.safePriorRevision)
    ? review.safePriorRevision
    : null;
  const sheet = useMemo(
    () => activeReview.sheets.find((item) => item.sheetId === sheetId) ?? activeReview.sheets[0] ?? null,
    [activeReview.sheets, sheetId],
  );

  const previewKey = sheet ? `${review.artifactId}:${activeReview.currentRevision}:${sheet.sheetId}` : "";
  const preview = previewState.key === previewKey ? previewState.source : null;
  const previewUnavailable = !sheet || sheet.previewPageCount < 1 || (previewState.key === previewKey && previewState.unavailable);
  useEffect(() => {
    let active = true;
    if (!sheet || sheet.previewPageCount < 1) {
      return () => { active = false; };
    }
    void workbookApi.preview(review.artifactId, activeReview.currentRevision, sheet.sheetId)
      .then((source) => { if (active) setPreviewState({ key: previewKey, source: source.dataUrl, unavailable: false }); })
      .catch(() => { if (active) setPreviewState({ key: previewKey, source: null, unavailable: true }); });
    return () => { active = false; };
  }, [activeReview.currentRevision, previewKey, review.artifactId, sheet]);

  if (!sheet) {
    return <p className="text-sm text-[var(--foreground-muted)]">{t("workbooks.no_sheet")}</p>;
  }

  async function revise() {
    if (!instruction.trim()) return;
    setBusy(true); setError("");
    try {
      const next = await workbookApi.revise({
        artifactId: review.artifactId,
        baseRevision: review.currentRevision,
        sheetId: sheet.sheetId,
        ...(cells.trim() ? { targetRange: cells.trim() } : {}),
        instruction: instruction.trim(),
      });
      onReplace(next); setActiveRevision(next.currentRevision); setCells(""); setInstruction(""); setSpecificCells(false);
    } catch (cause) {
      if (targetNeeded(cause)) {
        setSpecificCells(true);
        setDetailsOpen(true);
        setError("target_required");
        window.setTimeout(() => cellsInput.current?.focus(), 0);
      } else {
        setError("revision_failed");
      }
    } finally { setBusy(false); }
  }

  async function exportRevision() {
    if (!canExportWorkbook(activeReview)) return;
    setBusy(true); setError("");
    try { await workbookApi.exportRevision(review.artifactId, activeReview.currentRevision); }
    catch { setError("export_failed"); }
    finally { setBusy(false); }
  }

  const previewNode = <div><nav aria-label={t("workbooks.choose_sheet")} className="mb-3 flex flex-wrap gap-2">{activeReview.sheets.map((item) => <button aria-current={item.sheetId === sheet.sheetId ? "page" : undefined} className={`rounded border px-3 py-1.5 text-xs ${item.sheetId === sheet.sheetId ? "bg-[var(--fill-selected)]" : ""}`} key={item.sheetId} onClick={() => setSheetId(item.sheetId)} type="button">{item.name}</button>)}</nav>{preview ? <figure className="overflow-hidden rounded border bg-white p-2"><Image alt={t("workbooks.preview_alt", { sheet: sheet.name, revision: activeReview.currentRevision })} className="h-auto w-full" height={900} src={preview} unoptimized width={1440} /><figcaption className="mt-2 text-center text-xs text-[var(--foreground-muted)]">{t("workbooks.preview_caption", { sheet: sheet.name, revision: activeReview.currentRevision })}</figcaption></figure> : <p className="text-sm text-[var(--foreground-muted)]">{previewUnavailable ? t("workbooks.preview_unavailable") : t("workbooks.preview_loading")}</p>}</div>;

  function jumpToWarning(targetSheet: WorkbookSheetReview, ranges: string[]) {
    setSheetId(targetSheet.sheetId); setCells(ranges.join(", ")); setHighlightedCells(ranges); setSpecificCells(true); setDetailsOpen(true);
    window.setTimeout(() => {
      const target = document.getElementById(`workbook-numbers-${review.artifactId}`);
      if (target && typeof target.scrollIntoView === "function") {
        target.scrollIntoView({ behavior: "smooth", block: "center" });
      }
    }, 0);
  }

  return <DocumentReviewShell actions={<>{latest && failedCurrent && safePrior ? <button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)]" onClick={() => setActiveRevision(safePrior)} type="button">{t("workbook_labels.open_safe_version")}</button> : null}{!latest ? <button className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)]" onClick={() => setActiveRevision(review.currentRevision)} type="button">{t("workbooks.back_to_latest")}</button> : null}<button className="rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={busy || !canExportWorkbook(activeReview)} onClick={() => void exportRevision()} type="button">{t("workbooks.export")}</button></>} details={<div className="grid gap-6"><div id={`workbook-numbers-${review.artifactId}`}><NumberDetails highlightedCells={highlightedCells} sheet={sheet} /></div><OriginDetails sheet={sheet} /><RevisionHistory activeRevision={activeReview.currentRevision} onOpen={setActiveRevision} review={review} />{latest ? <div><h3 className="text-sm font-semibold">{t("workbooks.make_change")}</h3><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("workbooks.change_help", { sheet: sheet.name })}</p><textarea aria-label={t("workbooks.change_instructions")} className="mt-3 min-h-20 w-full rounded border bg-[var(--background)] p-2 text-sm" onChange={(event) => setInstruction(event.target.value)} placeholder={t("workbooks.instruction_placeholder")} value={instruction} />{specificCells ? <div className="mt-2"><label className="text-xs font-medium" htmlFor={`workbook-cells-${review.artifactId}`}>{t("workbooks.which_cells")}</label><input className="mt-1 block w-full rounded border bg-[var(--background)] p-2 text-sm" id={`workbook-cells-${review.artifactId}`} onChange={(event) => setCells(event.target.value)} placeholder={t("workbooks.cells_placeholder")} ref={cellsInput} value={cells} /></div> : <button className="mt-2 block text-xs font-semibold underline" onClick={() => setSpecificCells(true)} type="button">{t("workbooks.choose_specific_cells")}</button>}<button className="mt-3 rounded border px-3 py-2 text-xs font-semibold disabled:opacity-50" disabled={busy || !instruction.trim()} onClick={() => void revise()} type="button">{t("documents.save_new_version")}</button></div> : <p className="text-sm text-[var(--foreground-muted)]">{t("workbooks.latest_to_edit")}</p>}{error ? <p className="text-sm text-[var(--warning)]" role="alert">{t(`workbooks.errors.${error}`)}</p> : null}</div>} detailsId={`workbook-details-${review.artifactId}`} detailsOpen={detailsOpen} kind={t("documents.excel")} onDetailsToggle={setDetailsOpen} preview={previewNode} revision={activeReview.currentRevision} status={<WorkbookStatus review={activeReview} />} title={review.title} warnings={<WorkbookWarnings onJump={jumpToWarning} review={activeReview} />} />;
}

function WorkbookStatus({ review }: { review: WorkbookReview }) {
  const { t } = useI18n();
  const revision = review.revisions.find((item) => item.revision === review.currentRevision);
  const failed = revision?.status === "failed" || review.verification.status === "failed";
  const preparing = revision?.status === "building";
  const stale = review.calculation.status === "stale";
  const ready = review.verification.status === "verified" && canExportWorkbook(review);
  const title = preparing ? t("workbooks.status.preparing") : failed ? t("workbooks.status.failed") : stale ? t("workbooks.status.needs_recalculation") : ready ? t("workbooks.status.ready") : t("workbooks.status.checks_needed");
  const help = failed && review.safePriorRevision ? t("workbooks.status.previous_safe", { revision: review.safePriorRevision }) : preparing ? t("workbooks.status.preparing_help") : ready ? t("workbooks.checked") : t("workbooks.checks_needed");
  return <><p className="font-medium">{title}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{help}</p>{!canExportWorkbook(review) ? <p className="mt-1 text-xs font-medium">{t("workbooks.export_blocked")}</p> : null}</>;
}

function WorkbookWarnings({ onJump, review }: { onJump: (sheet: WorkbookSheetReview, ranges: string[]) => void; review: WorkbookReview }) {
  const { t } = useI18n();
  const warnings = review.sheets.flatMap((sheet) => sheet.warnings.map((warning) => ({ sheet, warning })));
  if (!warnings.length) return null;
  return <div><h3 className="text-sm font-semibold">{t("workbooks.needs_attention")}</h3><ul className="mt-2 grid gap-2">{warnings.map(({ sheet, warning }) => <li className="rounded bg-[var(--warning-background)] p-3 text-sm" key={`${sheet.sheetId}:${warning.warningId}`}><p>{t(`workbooks.warning_codes.${knownWarning(warning.code)}`, { sheet: sheet.name })}</p>{warning.ranges.length ? <button className="mt-1 text-xs font-semibold underline" onClick={() => onJump(sheet, warning.ranges)} type="button">{t("workbooks.show_cells", { sheet: sheet.name, cells: warning.ranges.join(", ") })}</button> : null}</li>)}</ul></div>;
}

function NumberDetails({ highlightedCells, sheet }: { highlightedCells: string[]; sheet: WorkbookSheetReview }) {
  const { t } = useI18n();
  return <div><h3 className="text-sm font-semibold">{t("workbooks.numbers")}</h3>{sheet.formulaCells.length ? <div className="mt-2 overflow-x-auto"><table className="w-full text-left text-xs"><thead><tr className="text-[var(--foreground-muted)]"><th className="p-2">{t("workbooks.cell")}</th><th className="p-2">{t("workbooks.rule")}</th><th className="p-2">{t("workbooks.result")}</th><th className="p-2">{t("workbooks.cell_status")}</th></tr></thead><tbody>{sheet.formulaCells.map((cell) => <tr className={`border-t border-[var(--border-soft)] ${highlightedCells.includes(cell.address) ? "bg-[var(--warning-background)]" : ""}`} key={cell.address}><td className="p-2 font-semibold">{cell.address}</td><td className="p-2">{cell.formula || t("workbooks.calculation_present")}</td><td className="p-2">{cell.displayValue || t("workbooks.result_pending")}</td><td className="p-2">{t(`workbooks.cell_states.${knownCellState(cell.status)}`)}</td></tr>)}</tbody></table></div> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("workbooks.no_calculations")}</p>}</div>;
}

function OriginDetails({ sheet }: { sheet: WorkbookSheetReview }) {
  const { t } = useI18n();
  return <div><h3 className="text-sm font-semibold">{t("workbooks.origins")}</h3>{sheet.lineage.length ? <ul className="mt-2 grid gap-2">{sheet.lineage.map((item) => <li className="rounded bg-[var(--accent-background)] p-2 text-xs" key={`${item.range}:${item.sourceId}`}><span className="font-semibold">{item.sourceLabel || t("workbook_labels.recorded_source")}</span><span className="mt-1 block text-[var(--foreground-muted)]">{item.range} · {t(`workbooks.freshness.${knownFreshness(item.freshness)}`)}</span></li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("workbooks.no_origins")}</p>}</div>;
}

function RevisionHistory({ activeRevision, onOpen, review }: { activeRevision: number; onOpen: (revision: number) => void; review: WorkbookReview }) {
  const { t } = useI18n();
  return <div><h3 className="text-sm font-semibold">{t("documents.versions")}</h3><ol className="mt-2 grid gap-2">{review.revisions.map((revision) => { const canOpen = revision.recoverable && review.versionReviews.some((item) => item.revision === revision.revision); return <li className="rounded border border-[var(--border-soft)] p-2 text-xs" key={revision.revision}><span className="font-semibold">{t("documents.revision", { revision: revision.revision })}</span><span className="mt-1 block text-[var(--foreground-muted)]">{t(`workbooks.revision_states.${knownRevision(revision.status)}`)}</span><span className="mt-1 block">{revision.recoverable ? t("workbooks.recoverable") : t("workbooks.recovery_unavailable")}</span>{revision.revision === activeRevision ? <span className="mt-2 block font-semibold">{t("workbooks.open_now")}</span> : canOpen ? <button className="mt-2 rounded border px-2 py-1 font-semibold" onClick={() => onOpen(revision.revision)} type="button">{t("workbooks.open_version")}</button> : null}</li>; })}</ol></div>;
}

function targetNeeded(error: unknown) {
  const code = typeof error === "object" && error && "code" in error ? String(error.code) : "";
  const message = error instanceof Error ? error.message : String(error);
  return ["workbook_revision_target_required", "workbook_revision_target_ambiguous"].some((value) => code === value || message.includes(value));
}

function knownCellState(value: string) { return ["calculated", "stale", "error", "unavailable"].includes(value) ? value : "unavailable"; }
function knownFreshness(value: string) { return ["fresh", "stale", "unknown"].includes(value) ? value : "unknown"; }
function knownRevision(value: string) { return ["verified", "stale", "failed", "building", "unavailable"].includes(value) ? value : "unavailable"; }
function knownWarning(value: string) { return ["column_content_clipped", "preview_truncated", "chart_data_missing", "formula_error", "critical_sheet_hidden", "needs_recalculation", "preview_unavailable", "preview_unsupported_characters", "package_relationship_invalid"].includes(value) ? value : "unknown"; }
