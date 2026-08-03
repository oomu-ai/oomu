"use client";

import { useCallback, useEffect, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { delegationApi, type DelegationPlan, type WorkSuggestion } from "./delegationClient";

const HELPER_STATE_KEYS: Record<string, string> = { planned: "helpers.state_waiting", running: "helpers.state_working", paused: "helpers.state_paused", completed: "helpers.state_done", failed: "helpers.state_retry", cancelled: "helpers.state_stopped", incomplete: "helpers.state_incomplete" };

export function ChildWorkstreams({ taskRunId }: { taskRunId: string }) {
  const { t } = useI18n();
  const [plans, setPlans] = useState<DelegationPlan[]>([]);
  const [suggestions, setSuggestions] = useState<Record<string, WorkSuggestion[]>>({});
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const next = await delegationApi.list(taskRunId); setPlans(next); setError("");
      const entries = await Promise.all(next.map(async (plan) => [plan.planId, await delegationApi.suggestions(plan.planId)] as const));
      setSuggestions(Object.fromEntries(entries));
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [taskRunId]);
  useEffect(() => { const timeout = window.setTimeout(() => void load(), 0); return () => window.clearTimeout(timeout); }, [load]);
  useEffect(() => { if (!plans.some((plan) => plan.state === "running")) return; const interval = window.setInterval(() => void load(), 1_000); return () => window.clearInterval(interval); }, [load, plans]);
  async function act(operation: Promise<unknown>) { try { await operation; await load(); } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); } }
  if (plans.length === 0 && !error) return null;
  return <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4">
    <div><h3 className="text-sm font-semibold">{t("helpers.title")}</h3><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t("helpers.subtitle")}</p></div>
    {error ? <p className="mt-3 text-xs text-[var(--warning)]" role="alert">{error}</p> : null}
    {plans.map((plan) => <Plan key={plan.planId} plan={plan} suggestions={suggestions[plan.planId] ?? []} act={act} t={t} />)}
  </section>;
}

function Plan({ plan, suggestions, act, t }: { plan: DelegationPlan; suggestions: WorkSuggestion[]; act: (operation: Promise<unknown>) => Promise<void>; t: (key: string, values?: Record<string, string | number>) => string }) {
  const done = plan.children.filter((child) => child.state === "completed").length;
  const disagreements = plan.synthesis?.uncertainties ?? [];
  return <div className="mt-4 border-t border-[var(--border-soft)] pt-4">
    {plan.synthesis?.findings.length ? <div className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-4"><h4 className="text-sm font-semibold">{t("helpers.findings")}</h4><ul className="mt-2 space-y-2 text-sm">{plan.synthesis.findings.map((finding, index) => <li key={index}>{finding.statement}</li>)}</ul></div> : null}
    {disagreements.length ? <div className="mt-3 rounded-[var(--radius-sm)] border border-[var(--warning)] bg-[var(--warning-background)] p-3"><p className="text-sm font-semibold">{t("helpers.disagreement_title")}</p><ul className="mt-1 list-disc pl-4 text-xs">{disagreements.map((item, index) => <li key={index}>{item}</li>)}</ul></div> : null}
    <div className="mt-3 flex flex-wrap items-center justify-between gap-2"><p className="text-sm font-semibold">{t("helpers.progress", { done, total: plan.children.length })}</p><div className="flex gap-2">{plan.state === "running" || plan.state === "planned" ? <><button className="rounded border px-2 py-1 text-xs" onClick={() => void act(delegationApi.pausePlan(plan.planId))} type="button">{t("helpers.pause_all")}</button><button className="rounded border px-2 py-1 text-xs" onClick={() => void act(delegationApi.cancelPlan(plan.planId))} type="button">{t("helpers.stop_all")}</button></> : null}{plan.state === "paused" ? <button className="rounded border px-2 py-1 text-xs" onClick={() => void act(delegationApi.resumePlan(plan.planId).then(() => delegationApi.execute(plan.planId)))} type="button">{t("helpers.continue_all")}</button> : null}{["completed", "partial"].includes(plan.state) ? <button className="rounded border px-2 py-1 text-xs font-semibold" onClick={() => void act(delegationApi.createDecisionBrief(plan.planId))} type="button">{t("helpers.make_brief")}</button> : null}</div></div>
    <div className="mt-3 divide-y divide-[var(--border-soft)]">{plan.children.map((helper) => <article className="py-3" key={helper.childRunId}><div className="flex items-start justify-between gap-3"><div><p className="text-sm font-medium">{helper.goal}</p><p className="mt-1 text-xs text-[var(--foreground-muted)]">{t(HELPER_STATE_KEYS[helper.state] ?? "helpers.state_waiting")}</p></div><div className="flex gap-1">{["running", "planned"].includes(helper.state) ? <button className="rounded border px-2 py-1 text-xs" onClick={() => void act(delegationApi.cancelChild(plan.planId, helper.childRunId))} type="button">{t("helpers.stop_one")}</button> : null}{["failed", "incomplete", "cancelled"].includes(helper.state) && helper.attempt < 4 ? <button className="rounded border px-2 py-1 text-xs" onClick={() => void act(delegationApi.retryChild(plan.planId, helper.childRunId))} type="button">{t("helpers.try_again")}</button> : null}</div></div>{helper.result ? <details className="mt-2 text-xs"><summary className="cursor-pointer font-semibold">{t("helpers.details")}</summary><ul className="mt-2 list-disc space-y-1 pl-4">{helper.result.findings.map((finding, index) => <li key={index}>{finding.statement}</li>)}</ul><p className="mt-2 text-[var(--foreground-muted)]">{t("helpers.sources_used", { count: helper.result.sources.length })}</p></details> : null}</article>)}</div>
    {suggestions.some((item) => item.state === "awaiting_review") ? <details className="mt-3"><summary className="cursor-pointer text-xs font-semibold">{t("helpers.suggestions")}</summary><div className="mt-2 space-y-2">{suggestions.filter((item) => item.state === "awaiting_review").map((item) => <div className="rounded border p-3 text-xs" key={item.suggestionId}><p>{item.summary}</p><div className="mt-2 flex gap-2"><button className="rounded border px-2 py-1" onClick={() => void act(delegationApi.reviewSuggestion(plan.planId, item.suggestionId, true))} type="button">{t("helpers.keep_suggestion")}</button><button className="rounded border px-2 py-1" onClick={() => void act(delegationApi.reviewSuggestion(plan.planId, item.suggestionId, false, t("helpers.left_out_reason")))} type="button">{t("helpers.leave_out")}</button></div></div>)}</div></details> : null}
    {isDeveloperBuild ? <details className="mt-3 text-xs"><summary className="cursor-pointer">{t("helpers.technical_details")}</summary><p className="mt-2 font-mono">{plan.children.length} / 8 · {plan.state}</p></details> : null}
  </div>;
}
