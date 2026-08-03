"use client";

import { useEffect, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { dismissFirstRunChatWelcome } from "../chat/firstRunWelcomeState";
import { projectApi, type ProjectRecord } from "../projects/projectClient";

type HeroRequirement = {
  id: string;
  label: string;
  state: string;
  detail: string;
  destination: string;
};

type HeroStatus = {
  readyOnDemand: boolean;
  readyWeekly: boolean;
  requirements: HeroRequirement[];
};

const heroRequirementIds = new Set([
  "project_knowledge",
  "instructions",
  "mail_calendar",
  "current_web",
  "parallel_research",
  "verified_artifact",
  "weekly_routine",
  "delivery",
]);

const heroDestinationIds = new Set([
  "projects",
  "integrations",
  "settings",
  "tasks",
  "artifacts",
  "routines",
]);

function requirementTranslationId(requirementId: string) {
  return heroRequirementIds.has(requirementId) ? requirementId : "generic";
}

function destinationLabelKey(destination: string) {
  return heroDestinationIds.has(destination)
    ? `hero.open_${destination}`
    : "hero.open_generic";
}

export function DecisionBriefScreen({
  onNavigate,
}: {
  onNavigate: (destination: string) => void;
}) {
  const { t } = useI18n();
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [projectsLoaded, setProjectsLoaded] = useState(false);
  const [projectId, setProjectId] = useState("");
  const [status, setStatus] = useState<HeroStatus | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);

  useEffect(() => {
    let active = true;
    void projectApi
      .list()
      .then((records) => {
        if (!active) return;
        setProjects(records);
        setProjectId((current) => current || records[0]?.projectId || "");
        setLoadFailed(false);
      })
      .catch(() => {
        if (active) setLoadFailed(true);
      })
      .finally(() => {
        if (active) setProjectsLoaded(true);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!projectId) return;
    let active = true;
    const timeout = window.setTimeout(() => {
      setStatus(null);
      setLoadFailed(false);
      void invoke<HeroStatus>("get_weekly_decision_brief_status", {
        request: { projectId },
        })
        .then((value) => {
          if (active) {
            setStatus(value);
            if (value.readyOnDemand || value.readyWeekly) {
              dismissFirstRunChatWelcome();
            }
          }
        })
        .catch(() => {
          if (active) setLoadFailed(true);
        });
    }, 0);
    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [projectId]);

  return (
    <section className="h-full overflow-y-auto p-7">
      <div className="mx-auto max-w-5xl">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h1 className="text-3xl font-semibold">{t("hero.title")}</h1>
            <p className="mt-2 max-w-3xl text-sm text-[var(--foreground-muted)]">
              {t("hero.subtitle")}
            </p>
          </div>
          {projects.length > 0 ? (
            <label className="grid gap-1 text-xs font-semibold">
              {t("hero.project")}
              <select
                className="min-w-56 rounded border bg-[var(--background)] px-3 py-2 text-sm font-normal"
                onChange={(event) => {
                  setProjectId(event.target.value);
                  setStatus(null);
                  setLoadFailed(false);
                }}
                value={projectId}
              >
                {projects.map((project) => (
                  <option key={project.projectId} value={project.projectId}>
                    {project.name}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
        </div>

        {loadFailed ? (
          <p
            className="mt-5 rounded bg-[var(--warning-background)] p-3 text-sm"
            role="alert"
          >
            {t("hero.load_error")}
          </p>
        ) : null}

        {status ? (
          <>
            <div className="mt-6 flex gap-2">
              <StatusBadge label={t("hero.on_demand")} ready={status.readyOnDemand} />
              <StatusBadge label={t("hero.weekly")} ready={status.readyWeekly} />
            </div>
            <p className="mt-4 rounded bg-[var(--accent-background)] p-4 text-sm text-[var(--foreground-muted)]">
              {t("hero.evidence")}
            </p>
            <ol className="mt-6 grid gap-3 sm:grid-cols-2">
              {status.requirements.map((item, index) => {
                const ready = item.state === "ready";
                const requirementId = requirementTranslationId(item.id);
                return (
                  <li
                    className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4"
                    key={`${item.id}-${index}`}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <p className="text-[10px] font-semibold uppercase tracking-wide text-[var(--foreground-subtle)]">
                          {index + 1}
                        </p>
                        <h2 className="mt-1 text-sm font-semibold">
                          {t(`hero.requirements.${requirementId}.label`)}
                        </h2>
                      </div>
                      <span
                        className={`rounded-full px-2 py-1 text-[10px] font-semibold ${
                          ready
                            ? "bg-[var(--success-background)]"
                            : "bg-[var(--warning-background)]"
                        }`}
                      >
                        {ready ? t("hero.ready") : t("hero.setup")}
                      </span>
                    </div>
                    <p className="mt-2 text-xs text-[var(--foreground-muted)]">
                      {t(`hero.requirements.${requirementId}.${ready ? "ready" : "setup"}`)}
                    </p>
                    <button
                      className="mt-3 text-xs font-semibold underline"
                      onClick={() => onNavigate(item.destination)}
                      type="button"
                    >
                      {t(destinationLabelKey(item.destination))}
                    </button>
                  </li>
                );
              })}
            </ol>
          </>
        ) : !loadFailed && projectId ? (
          <p className="mt-6 text-sm text-[var(--foreground-muted)]">
            {t("hero.loading")}
          </p>
        ) : !loadFailed && projectsLoaded ? (
          <div className="mt-6 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4">
            <p className="text-sm text-[var(--foreground-muted)]">{t("hero.no_projects")}</p>
            <button
              className="mt-3 text-xs font-semibold underline"
              onClick={() => onNavigate("projects")}
              type="button"
            >
              {t("hero.open_projects")}
            </button>
          </div>
        ) : (
          <p className="mt-6 text-sm text-[var(--foreground-muted)]">
            {t("hero.loading")}
          </p>
        )}
      </div>
    </section>
  );
}

function StatusBadge({ label, ready }: { label: string; ready: boolean }) {
  const { t } = useI18n();
  return (
    <span
      className={`rounded-full px-3 py-1 text-xs font-semibold ${
        ready ? "bg-[var(--success-background)]" : "bg-[var(--warning-background)]"
      }`}
    >
      {label}: {ready ? t("hero.ready") : t("hero.setup")}
    </span>
  );
}
