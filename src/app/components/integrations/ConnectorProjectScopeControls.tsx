"use client";

import { useState } from "react";
import { useI18n } from "@/context/I18nContext";
import type { ProjectRecord } from "../projects/projectClient";
import type { ConnectorProjectScope } from "./integrationClient";

type ScopeAccount = {
  connectorId: string;
  connectionState: string;
  allProjectsEnabled: boolean;
  projectScopeReviewedAtMs?: number | null;
  enabledProjectIds: string[];
};

const READY_STATES = new Set(["authorized", "reachable"]);

export function ConnectorProjectScopeControls({
  account,
  disabled = false,
  projects,
  saveScope,
}: {
  account: ScopeAccount;
  disabled?: boolean;
  projects: ProjectRecord[];
  saveScope: (allProjectsEnabled: boolean, enabledProjectIds: string[]) => Promise<ConnectorProjectScope>;
}) {
  const persistedSignature = [...account.enabledProjectIds].sort().join("\u0000");
  const persistedSelection = persistedSignature ? persistedSignature.split("\u0000") : [];
  const nativeProjectionKey = [
    account.connectorId,
    account.connectionState,
    account.allProjectsEnabled ? "all" : "selected",
    account.projectScopeReviewedAtMs ?? "unreviewed",
    persistedSignature,
  ].join(":");
  return <ProjectScopeState
    account={account}
    disabled={disabled}
    key={nativeProjectionKey}
    persistedSelection={persistedSelection}
    projects={projects}
    saveScope={saveScope}
  />;
}

function ProjectScopeState({
  account,
  disabled,
  persistedSelection,
  projects,
  saveScope,
}: {
  account: ScopeAccount;
  disabled: boolean;
  persistedSelection: string[];
  projects: ProjectRecord[];
  saveScope: (allProjectsEnabled: boolean, enabledProjectIds: string[]) => Promise<ConnectorProjectScope>;
}) {
  const { t } = useI18n();
  const [nativeScope, setNativeScope] = useState({
    allProjectsEnabled: account.allProjectsEnabled,
    enabledProjectIds: persistedSelection,
    reviewed: account.projectScopeReviewedAtMs != null,
  });
  const [allProjectsEnabled, setAllProjectsEnabled] = useState(
    account.projectScopeReviewedAtMs == null && READY_STATES.has(account.connectionState)
      ? true
      : account.allProjectsEnabled,
  );
  const [enabledProjectIds, setEnabledProjectIds] = useState(persistedSelection);
  const [state, setState] = useState<"idle" | "working" | "success" | "error">("idle");
  const ready = READY_STATES.has(account.connectionState);

  const selectionSignature = [...enabledProjectIds].sort().join("\u0000");
  const dirty = allProjectsEnabled !== nativeScope.allProjectsEnabled
    || selectionSignature !== nativeScope.enabledProjectIds.join("\u0000")
    || !nativeScope.reviewed;

  function toggleProject(projectId: string) {
    setEnabledProjectIds((current) => current.includes(projectId)
      ? current.filter((id) => id !== projectId)
      : [...current, projectId].sort());
    setState("idle");
  }

  async function save() {
    if (!ready || disabled || state === "working" || !dirty) return;
    setState("working");
    try {
      const persisted = await saveScope(allProjectsEnabled, enabledProjectIds);
      const next = {
        allProjectsEnabled: persisted.allProjectsEnabled,
        enabledProjectIds: [...persisted.enabledProjectIds].sort(),
        reviewed: true,
      };
      setNativeScope(next);
      setAllProjectsEnabled(next.allProjectsEnabled);
      setEnabledProjectIds(next.enabledProjectIds);
      setState("success");
    } catch {
      setAllProjectsEnabled(nativeScope.allProjectsEnabled);
      setEnabledProjectIds(nativeScope.enabledProjectIds);
      setState("error");
    }
  }

  const controlsDisabled = disabled || !ready || state === "working";
  return <fieldset className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4" disabled={controlsDisabled}>
    <legend className="px-1 text-sm font-semibold">{t("connector_project_scope.title")}</legend>
    <label className="flex cursor-pointer items-start gap-3">
      <input
        checked={allProjectsEnabled}
        className="mt-0.5 size-4"
        onChange={(event) => { setAllProjectsEnabled(event.target.checked); setState("idle"); }}
        type="checkbox"
      />
      <span><span className="block text-sm font-medium">{t("connector_project_scope.all_projects")}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{t("connector_project_scope.all_projects_help")}</span></span>
    </label>
    {!allProjectsEnabled ? <div className="mt-4 border-t border-[var(--border-soft)] pt-4">
      <p className="text-sm font-semibold">{t("connector_project_scope.choose_projects")}</p>
      {projects.length ? <ul className="mt-2 grid gap-2">{projects.map((project) => <li key={project.projectId}>
        <label className="flex cursor-pointer items-center gap-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-sm">
          <input checked={enabledProjectIds.includes(project.projectId)} onChange={() => toggleProject(project.projectId)} type="checkbox" />
          <span>{project.name}</span>
        </label>
      </li>)}</ul> : <p className="mt-2 text-xs text-[var(--foreground-muted)]">{t("connector_project_scope.no_projects_narrow")}</p>}
    </div> : projects.length === 0 ? <p className="mt-3 text-xs text-[var(--foreground-muted)]">{t("connector_project_scope.no_projects_all")}</p> : null}
    {!ready ? <p className="mt-3 text-xs text-[var(--foreground-muted)]">{t("connector_project_scope.reconnect_first")}</p> : null}
    <div className="mt-4 flex flex-wrap items-center gap-3">
      <button
        aria-busy={state === "working"}
        className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
        data-action-state={state}
        disabled={controlsDisabled || !dirty}
        onClick={() => void save()}
        type="button"
      >{state === "working" ? t("connector_project_scope.saving") : t("connector_project_scope.save")}</button>
      <span aria-live="polite" className={state === "error" ? "text-sm text-[var(--destructive)]" : "text-sm text-[var(--foreground-muted)]"} role={state === "error" ? "alert" : "status"}>
        {state === "success" ? t("connector_project_scope.saved") : state === "error" ? t("connector_project_scope.failed") : ""}
      </span>
    </div>
  </fieldset>;
}
