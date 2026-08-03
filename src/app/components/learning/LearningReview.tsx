"use client";

import { useCallback, useEffect, useState } from "react";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useApprovalDialogTurn } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import { learningApi, type LearningOffer, type SavedMethod } from "./learningClient";

export function LearningReview({
  projectId,
  taskRunId,
  completed,
}: {
  projectId: string;
  taskRunId: string;
  completed: boolean;
}) {
  const { t } = useI18n();
  const [offers, setOffers] = useState<LearningOffer[]>([]);
  const [methods, setMethods] = useState<SavedMethod[]>([]);
  const [offerText, setOfferText] = useState("");
  const [methodEditId, setMethodEditId] = useState("");
  const [methodText, setMethodText] = useState("");
  const [search, setSearch] = useState("");
  const [error, setError] = useState("");
  const [forgotten, setForgotten] = useState("");
  const [confirmEverywhere, setConfirmEverywhere] = useState(false);
  const [savingEverywhere, setSavingEverywhere] = useState(false);

  const load = useCallback(async () => {
    try {
      const [nextOffers, nextMethods] = await Promise.all([
        learningApi.offers(taskRunId),
        learningApi.methods(projectId),
      ]);
      setOffers(nextOffers);
      setMethods(nextMethods);
      setError("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [projectId, taskRunId]);

  useEffect(() => {
    const timeout = window.setTimeout(() => void load(), 0);
    return () => window.clearTimeout(timeout);
  }, [load]);

  async function act(operation: Promise<unknown>) {
    try {
      await operation;
      await load();
      setError("");
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return false;
    }
  }

  const active = offers.find(
    (offer) => offer.status === "proposed" || offer.status === "postponed",
  );
  const shownMethods = methods.filter((method) =>
    method.name.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()),
  );
  const proposedMethod = active ? offerText || active.summary : "";
  const confirmationDialogId = `learning-everywhere:${active?.offerId ?? "none"}`;
  const hasConfirmationTurn = useApprovalDialogTurn(
    confirmEverywhere && Boolean(active),
    confirmationDialogId,
  );
  const permissionMethodPreview = sanitizePermissionPromptText(proposedMethod);

  async function saveEverywhereChoice() {
    if (!active || savingEverywhere) return;
    setSavingEverywhere(true);
    const saved = await act(
      learningApi.review(
        active.offerId,
        "remember_everywhere",
        offerText || undefined,
        true,
      ),
    );
    if (saved) setConfirmEverywhere(false);
    setSavingEverywhere(false);
  }

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4">
      {active ? (
        <div>
          <h3 className="text-sm font-semibold">{t("learning.offer_title")}</h3>
          <p className="mt-2 text-sm">{proposedMethod}</p>
          <p className="mt-2 text-xs text-[var(--foreground-muted)]">
            {t("learning.from_tasks", { count: active.sourceTaskCount })}{" "}
            {active.exposureSummary}
          </p>
          {offerText ? (
            <textarea
              aria-label={t("learning.edit_label")}
              className="mt-3 w-full rounded border p-2 text-sm"
              onChange={(event) => setOfferText(event.target.value)}
              value={offerText}
            />
          ) : null}
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              className="rounded bg-[var(--accent)] px-3 py-2 text-xs font-semibold text-white"
              onClick={() =>
                void act(
                  learningApi.review(
                    active.offerId,
                    "remember_project",
                    offerText || undefined,
                  ),
                )
              }
              type="button"
            >
              {t("learning.just_project")}
            </button>
            <button
              className="rounded border px-3 py-2 text-xs"
              onClick={() => setConfirmEverywhere(true)}
              type="button"
            >
              {t("permissions.use_everywhere")}
            </button>
            <button
              className="rounded border px-3 py-2 text-xs"
              onClick={() => setOfferText(offerText ? "" : active.summary)}
              type="button"
            >
              {t("learning.tweak")}
            </button>
            <button
              className="rounded border px-3 py-2 text-xs"
              onClick={() =>
                void act(learningApi.review(active.offerId, "no_thanks"))
              }
              type="button"
            >
              {t("learning.no_thanks")}
            </button>
            <button
              className="rounded border px-3 py-2 text-xs"
              onClick={() =>
                void act(learningApi.review(active.offerId, "ask_later"))
              }
              type="button"
            >
              {t("learning.ask_later")}
            </button>
          </div>
        </div>
      ) : completed ? (
        <div>
          <h3 className="text-sm font-semibold">{t("learning.title")}</h3>
          <p className="mt-1 text-xs text-[var(--foreground-muted)]">
            {t("learning.subtitle")}
          </p>
          <button
            className="mt-3 rounded border px-3 py-2 text-xs font-semibold"
            onClick={() => void act(learningApi.prepare(taskRunId))}
            type="button"
          >
            {t("learning.review_button")}
          </button>
        </div>
      ) : null}

      {methods.length ? (
        <details className="mt-4">
          <summary className="cursor-pointer text-xs font-semibold">
            {t("learning.saved_methods", { count: methods.length })}
          </summary>
          <input
            aria-label={t("learning.search")}
            className="mt-3 w-full rounded border px-3 py-2 text-sm"
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t("learning.search")}
            value={search}
          />
          <div className="mt-2 space-y-2">
            {shownMethods.map((method) => (
              <article
                className="rounded bg-[var(--accent-background)] p-3"
                key={method.methodId}
              >
                {methodEditId === method.methodId ? (
                  <>
                    <textarea
                      aria-label={t("learning.edit_label")}
                      className="w-full rounded border p-2 text-sm"
                      onChange={(event) => setMethodText(event.target.value)}
                      value={methodText}
                    />
                    <div className="mt-2 flex gap-2">
                      <button
                        className="rounded border px-2 py-1 text-xs font-semibold"
                        onClick={() => {
                          void act(learningApi.edit(method.methodId, methodText));
                          setMethodEditId("");
                        }}
                        type="button"
                      >
                        {t("common.save")}
                      </button>
                      <button
                        className="rounded border px-2 py-1 text-xs"
                        onClick={() => setMethodEditId("")}
                        type="button"
                      >
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="text-sm font-medium">{method.name}</p>
                    <p className="mt-1 text-xs text-[var(--foreground-muted)]">
                      {method.projectId
                        ? t("learning.this_project")
                        : t("learning.all_projects")}
                      {method.useCount
                        ? t("learning.saved_steps", {
                            success: method.successfulUseCount,
                            total: method.useCount,
                          })
                        : ""}
                    </p>
                    <div className="mt-2 flex flex-wrap gap-2">
                      <button
                        className="rounded border px-2 py-1 text-xs"
                        onClick={() =>
                          void act(
                            learningApi.setEnabled(
                              method.methodId,
                              !method.enabled,
                            ),
                          )
                        }
                        type="button"
                      >
                        {method.enabled
                          ? t("learning.turn_off")
                          : t("learning.turn_on")}
                      </button>
                      <button
                        className="rounded border px-2 py-1 text-xs"
                        onClick={() => {
                          setMethodEditId(method.methodId);
                          setMethodText(method.summary);
                        }}
                        type="button"
                      >
                        {t("common.edit")}
                      </button>
                      {method.history.length > 1 ? (
                        <button
                          className="rounded border px-2 py-1 text-xs"
                          onClick={() =>
                            void act(
                              learningApi.goBack(
                                method.methodId,
                                method.history[1].version,
                              ),
                            )
                          }
                          type="button"
                        >
                          {t("learning.go_back")}
                        </button>
                      ) : null}
                      <button
                        className="rounded border px-2 py-1 text-xs"
                        onClick={() => {
                          setForgotten(method.methodId);
                          void act(learningApi.forget(method.methodId));
                        }}
                        type="button"
                      >
                        {t("learning.forget")}
                      </button>
                    </div>
                    <details className="mt-2 text-xs">
                      <summary className="cursor-pointer">
                        {t("learning.history")}
                      </summary>
                      <ol className="mt-1 space-y-1">
                        {method.history.map((version) => (
                          <li key={version.version}>
                            {new Date(version.createdAtMs).toLocaleDateString()} ·{" "}
                            {version.summary}
                          </li>
                        ))}
                      </ol>
                    </details>
                  </>
                )}
              </article>
            ))}
          </div>
        </details>
      ) : null}

      {forgotten ? (
        <div className="mt-3 flex items-center justify-between rounded bg-[var(--accent-background)] p-2 text-xs">
          <span>{t("learning.forgotten")}</span>
          <button
            className="font-semibold"
            onClick={() => {
              void act(learningApi.undo(forgotten));
              setForgotten("");
            }}
            type="button"
          >
            {t("common.undo")}
          </button>
        </div>
      ) : null}
      {error ? (
        <p className="mt-3 text-xs text-[var(--warning)]" role="alert">
          {error}
        </p>
      ) : null}

      {confirmEverywhere && active && hasConfirmationTurn ? (
        <ApprovalDialogFrame
          description={
            <>
              <p>{t("permissions.learn_everywhere_detail")}</p>
              <p className="mt-2">{t("permissions.learn_everywhere_reason")}</p>
            </>
          }
          eyebrow={t("permissions.review_choice")}
          footer={
            <>
              <button
                className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                data-approval-initial-focus
                disabled={savingEverywhere}
                onClick={() => setConfirmEverywhere(false)}
                type="button"
              >
                {t("permissions.not_now")}
              </button>
              <button
                aria-busy={savingEverywhere}
                className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
                data-action-state={savingEverywhere ? "working" : "idle"}
                disabled={savingEverywhere}
                onClick={() => void saveEverywhereChoice()}
                type="button"
              >
                {savingEverywhere
                  ? t("permissions.saving_choice")
                  : t("permissions.use_everywhere")}
              </button>
            </>
          }
          onDismiss={() => {
            if (!savingEverywhere) setConfirmEverywhere(false);
          }}
          title={t("permissions.learn_everywhere_title")}
        >
          <p className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-3 text-sm leading-6">
            {permissionMethodPreview || t("learning.offer_title")}
          </p>
        </ApprovalDialogFrame>
      ) : null}
    </section>
  );
}

export function sanitizePermissionPromptText(value: string, maxLength = 240) {
  const plainText = value
    .replace(/(?:```|~~~)[\s\S]*?(?:```|~~~)/g, " ")
    .replace(/(?:```|~~~)[\s\S]*$/g, " ")
    .replace(/`[^`]*`/g, " ")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])\s+/gm, "")
    .replace(/^\s*\[[ xX]\]\s+/gm, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/[\u0000-\u001F\u007F]/g, " ")
    .replace(/[*_~]/g, "")
    .replace(/\bhttps?:\/\/\S+/gi, "")
    .replace(/\s+/g, " ")
    .trim();

  if (plainText.length <= maxLength) return plainText;
  const shortened = plainText.slice(0, Math.max(1, maxLength - 1)).trimEnd();
  return `${shortened}…`;
}
