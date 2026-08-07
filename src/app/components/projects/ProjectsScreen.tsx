"use client";

import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { projectApi, type ProjectPolicy, type ProjectRecord, type ProjectSource } from "./projectClient";
import { ScreenEmptyState } from "../shared/ScreenEmptyState";
import type { WorkflowProjectScope } from "@/components/AppShell";
import { ProjectFolderPanel } from "./ProjectFolderPanel";

type TranslateFn = (key: string, values?: Record<string, string | number>) => string;

export function projectSourceStateLabel(t: TranslateFn, state: string) {
  switch (state) {
    case "ready":
    case "indexed":
      return t("projects.source_state_ready");
    case "pending":
      return t("projects.source_state_pending");
    case "indexing":
      return t("projects.source_state_indexing");
    case "failed":
    case "revoked":
      return t("projects.source_state_attention");
    default:
      return t("projects.source_state_unknown");
  }
}

type PickerResult = {
  grantId: string;
  directoryName: string;
  canonicalPath: string;
  fileCount: number;
};
type MemorySummary = { memoryCount: number; sourceSessions: string[] };
type PendingProjectDeletion = { projectId: string; name: string };

export function ProjectsScreen({ onOpenChat: openProjectChat, onOpenWorkflows: openProjectWorkflows }: { onOpenChat: (projectId: string) => void; onOpenWorkflows: (scope: WorkflowProjectScope) => void }) {
  const { t } = useI18n();
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [sources, setSources] = useState<ProjectSource[]>([]);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [policy, setPolicy] = useState<ProjectPolicy>("ask_before_cloud");
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [memory, setMemory] = useState<MemorySummary>({ memoryCount: 0, sourceSessions: [] });
  const [deletePreparing, setDeletePreparing] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const [pendingDeletion, setPendingDeletion] = useState<PendingProjectDeletion | null>(null);
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const selected = projects.find((project) => project.projectId === selectedId) ?? null;
  const projectFolder = sources.find((source) => source.sourceKind === "local_folder" && source.grantState === "active") ?? null;
  const knowledgeSources = sources.filter((source) => source.sourceKind === "knowledge_directory");
  const onOpenChat = () => {
    if (selected) openProjectChat(selected.projectId);
  };
  const onOpenWorkflows = () => {
    if (selected) {
      openProjectWorkflows({
        projectId: selected.projectId,
        projectName: selected.name,
      });
    }
  };
  const loadProjects = useCallback(async () => {
    try {
      const records = await projectApi.list();
      setProjects(records);
      setSelectedId((current) => records.some((project) => project.projectId === current) ? current : records[0]?.projectId ?? "");
      setError("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);
  useEffect(() => {
    const timeout = window.setTimeout(() => void loadProjects(), 0);
    return () => window.clearTimeout(timeout);
  }, [loadProjects]);
  useEffect(() => {
    const timeout = window.setTimeout(() => {
      if (!selected) { setSources([]); return; }
      setName(selected.name);
      setDescription(selected.description);
      setInstructions(selected.instructions);
      setPolicy(selected.dataPolicy);
      void projectApi.sources(selected.projectId).then(setSources).catch((cause) => setError(String(cause)));
      void invoke<MemorySummary>("get_project_memory_summary", { request: { projectId: selected.projectId } }).then(setMemory).catch(() => setMemory({ memoryCount: 0, sourceSessions: [] }));
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [selected]);

  async function run(operation: () => Promise<unknown>, message: string) {
    setBusy(true); setError(""); setNotice("");
    try { await operation(); await loadProjects(); setNotice(message); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  }

  async function createProject() {
    if (!name.trim()) return;
    await run(async () => {
      const created = await projectApi.create(name, description, policy);
      setSelectedId(created.projectId); setCreating(false);
    }, t("projects.created"));
  }

  async function addKnowledgeSource() {
    if (!selected) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const scope = `project-${Date.now()}`;
      const picked = await invoke<PickerResult | null>("choose_knowledge_ingest_directory", { request: { sessionId: scope, turnId: scope, maxFiles: 240 } });
      if (!picked) return;
      await invoke("attach_project_source", { request: { projectId: selected.projectId, path: picked.canonicalPath, grantReference: picked.grantId, sourceKind: "knowledge_directory" } });
      await invoke("ingest_knowledge", { request: { grantId: picked.grantId, sessionId: scope, turnId: scope, projectId: selected.projectId } });
      const refreshed = await projectApi.sources(selected.projectId);
      setSources(refreshed);
      const pending = refreshed.find((source) => source.canonicalPath === picked.canonicalPath);
      if (pending) await projectApi.refreshSource(selected.projectId, pending.sourceId);
      setSources(await projectApi.sources(selected.projectId));
      await loadProjects();
      setNotice(t(picked.fileCount === 0 ? "projects.source_added_empty" : "projects.source_added"));
    } catch {
      setError(t("projects.source_add_failed"));
    } finally {
      setBusy(false);
    }
  }

  async function openDeleteConfirmation() {
    if (!selected) return;
    setDeletePreparing(true);
    setDeleteError("");
    setError("");
    setNotice("");
    try {
      await projectApi.previewDeletion(selected.projectId);
      setPendingDeletion({ projectId: selected.projectId, name: selected.name });
    } catch {
      setError(t("projects.delete_failed"));
    } finally {
      setDeletePreparing(false);
    }
  }

  function closeDeleteConfirmation() {
    if (deleteBusy) return;
    setDeleteError("");
    setPendingDeletion(null);
    window.setTimeout(() => deleteTriggerRef.current?.focus(), 0);
  }

  async function deleteProject() {
    if (!pendingDeletion) return;
    const projectId = pendingDeletion.projectId;
    setDeleteBusy(true);
    setDeleteError("");
    setError("");
    setNotice("");
    try {
      await projectApi.delete(projectId);
      setPendingDeletion(null);
      await loadProjects();
      setNotice(t("projects.deleted"));
    } catch {
      setDeleteError(t("projects.delete_failed"));
    } finally {
      setDeleteBusy(false);
    }
  }

  if (error && projects.length === 0) {
    return <section className="flex h-full items-center justify-center p-8"><div role="alert" className="max-w-lg rounded-[var(--radius-md)] border border-[var(--border-strong)] p-6"><h1 className="text-xl font-semibold">{t("projects.unavailable_title")}</h1><p className="mt-2 text-sm text-[var(--foreground-muted)]">{error}</p><button className="mt-4 rounded-[var(--radius-sm)] border px-3 py-2 text-sm" onClick={() => void loadProjects()} type="button">{t("common.refresh")}</button></div></section>;
  }

  return (
    <section className="grid h-full min-h-0 grid-cols-[19rem_minmax(0,1fr)] overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)]">
      <aside aria-hidden={pendingDeletion ? true : undefined} className="flex min-h-0 flex-col border-r border-[var(--border-soft)] p-4">
        <div className="flex items-center justify-between gap-3">
          <h2 className="text-sm font-semibold text-[var(--foreground)]">{t("projects.list_label")}</h2>
          <button className="shrink-0 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-1.5 text-xs font-semibold text-[var(--inverse-foreground)] transition-opacity hover:opacity-90" onClick={() => { setCreating(true); setSelectedId(""); setName(""); setDescription(""); }} type="button">{t("projects.new")}</button>
        </div>
        <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">{t("projects.definition")}</p>
        <div className="mt-4 flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
          {projects.length === 0 && !creating ? <ScreenEmptyState actionLabel={t("projects.create_first")} body={t("projects.empty")} icon={<ProjectEmptyIcon />} onAction={() => setCreating(true)} title={t("projects.empty_title")} /> : null}
          {projects.map((project) => <button aria-current={selectedId === project.projectId ? "page" : undefined} className={`rounded-[var(--radius-sm)] px-3 py-3 text-left ${selectedId === project.projectId ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"}`} key={project.projectId} onClick={() => { setCreating(false); setSelectedId(project.projectId); }} type="button"><span className="block text-sm font-semibold">{project.name}</span><span className="mt-1 block text-xs text-[var(--foreground-muted)]">{t("projects.source_count", { count: project.sourceCount })}</span></button>)}
        </div>
      </aside>
      <div aria-hidden={pendingDeletion ? true : undefined} className="min-h-0 overflow-y-auto p-8">
        {creating ? <div className="mx-auto flex max-w-2xl flex-col gap-5"><h2 className="text-2xl font-semibold">{t("projects.create_title")}</h2><ProjectFields {...{name, setName, description, setDescription, instructions, setInstructions, policy, setPolicy, t}} hideInstructions /><button className="w-fit rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50" disabled={busy || !name.trim()} onClick={() => void createProject()} type="button">{t("projects.create")}</button></div> : selected ? <div className="mx-auto flex max-w-4xl flex-col gap-7"><div><h2 className="text-2xl font-semibold">{selected.name}</h2><p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("projects.summary", { conversations: selected.conversationCount, workflows: selected.workflowCount, tasks: selected.taskCount })}</p></div><ProjectFields {...{name, setName, description, setDescription, instructions, setInstructions, policy, setPolicy, t}} /><div className="flex flex-wrap gap-2"><button className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)]" disabled={busy} onClick={() => void run(async () => { await projectApi.update(selected.projectId, name, description); await projectApi.instructions(selected.projectId, instructions); await projectApi.policy(selected.projectId, policy); }, t("projects.saved"))} type="button">{t("common.save")}</button><button className="rounded-[var(--radius-sm)] border px-4 py-2 text-sm" onClick={onOpenChat} type="button">{t("projects.open_conversations")}</button><button className="rounded-[var(--radius-sm)] border px-4 py-2 text-sm" onClick={onOpenWorkflows} type="button">{t("projects.open_workflows")}</button><button className="rounded-[var(--radius-sm)] border px-4 py-2 text-sm" disabled={busy} onClick={() => void run(() => projectApi.archive(selected.projectId), t("projects.archived"))} type="button">{t("projects.archive")}</button><button aria-busy={deletePreparing} className="rounded-[var(--radius-sm)] border border-[var(--destructive)] px-4 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:opacity-50" disabled={busy || deletePreparing || deleteBusy} onClick={() => void openDeleteConfirmation()} ref={deleteTriggerRef} type="button">{deletePreparing ? t("projects.preparing_delete") : t("projects.delete")}</button></div><div className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-4"><h3 className="text-sm font-semibold">{t("projects.memory_title")}</h3><p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("projects.memory_summary", { count: memory.memoryCount, sources: memory.sourceSessions.length })}</p></div><ProjectFolderPanel busy={busy} folder={projectFolder} projectId={selected.projectId} onChanged={async (next) => { setSources(next); await loadProjects(); setNotice(t("projects.folder_saved")); }} onFailed={() => setError(t("projects.folder_failed"))} t={t} />
        <div className="border-t border-[var(--border-soft)] pt-6"><div className="flex items-start justify-between gap-5"><div><h3 className="font-semibold">{t("projects.knowledge_title")}</h3><p className="mt-1 text-sm text-[var(--foreground-muted)]">{t("projects.knowledge_help")}</p><p className="mt-2 max-w-2xl text-xs leading-5 text-[var(--foreground-muted)]">{t("projects.knowledge_limits")}</p></div><button className="shrink-0 rounded-[var(--radius-sm)] border px-3 py-2 text-sm font-semibold" disabled={busy} onClick={() => void addKnowledgeSource()} type="button">{t("projects.add_source")}</button></div><div className="mt-4 flex flex-col gap-2">{knowledgeSources.length === 0 ? <p className="rounded-[var(--radius-sm)] border border-dashed p-4 text-sm text-[var(--foreground-muted)]">{t("projects.no_sources")}</p> : knowledgeSources.map((source) => <SourceRow key={source.sourceId} projectId={selected.projectId} source={source} setError={setError} setSources={setSources} t={t} />)}</div></div>{notice ? <p aria-live="polite" className="text-sm text-[var(--success)]">{notice}</p> : null}{error ? <p role="alert" className="text-sm text-[var(--warning)]">{error}</p> : null}</div> : <div className="flex h-full items-center justify-center text-sm text-[var(--foreground-muted)]">{t("projects.select")}</div>}
      </div>
      {pendingDeletion ? <ProjectDeleteDialog busy={deleteBusy} error={deleteError} name={pendingDeletion.name} onCancel={closeDeleteConfirmation} onConfirm={() => void deleteProject()} t={t} /> : null}
    </section>
  );
}

function ProjectEmptyIcon() {
  return <svg aria-hidden="true" className="h-8 w-8" fill="none" stroke="currentColor" strokeWidth="1.5" viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3z" /><path d="M3 10h18" /></svg>;
}

function ProjectDeleteDialog({ busy, error, name, onCancel, onConfirm, t }: { busy: boolean; error: string; name: string; onCancel: () => void; onConfirm: () => void; t: TranslateFn }) {
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useLayoutEffect(() => {
    if (busy) dialogRef.current?.focus();
    else cancelRef.current?.focus();
  }, [busy]);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const buttons = Array.from(dialogRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
    if (buttons.length === 0) {
      event.preventDefault();
      dialogRef.current?.focus();
      return;
    }
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"><div aria-busy={busy} aria-describedby={descriptionId} aria-labelledby={titleId} aria-modal="true" className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-6 shadow-[var(--shadow-raised)]" onKeyDown={handleKeyDown} ref={dialogRef} role="dialog" tabIndex={-1}><h3 className="text-lg font-semibold" id={titleId}>{t("projects.delete_title", { name })}</h3><div className="mt-3 grid gap-2 text-sm text-[var(--foreground-muted)]" id={descriptionId}><p>{t("projects.delete_warning")}</p><p>{t("projects.delete_linked_folders")}</p><p>{t("projects.delete_preserved_work")}</p></div>{error ? <p className="mt-4 text-sm text-[var(--destructive)]" role="alert">{error}</p> : null}<div className="mt-6 flex justify-end gap-2"><button className="rounded-[var(--radius-sm)] border px-3 py-2 text-sm" disabled={busy} onClick={onCancel} ref={cancelRef} type="button">{t("common.cancel")}</button><button className="rounded-[var(--radius-sm)] bg-[var(--destructive)] px-3 py-2 text-sm font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-50" disabled={busy} onClick={onConfirm} type="button">{busy ? t("projects.deleting") : t("projects.delete_confirm")}</button></div></div></div>;
}

function ProjectFields({ name, setName, description, setDescription, instructions, setInstructions, policy, setPolicy, t, hideInstructions = false }: { name: string; setName: (value: string) => void; description: string; setDescription: (value: string) => void; instructions: string; setInstructions: (value: string) => void; policy: ProjectPolicy; setPolicy: (value: ProjectPolicy) => void; t: (key: string, values?: Record<string, string | number>) => string; hideInstructions?: boolean }) {
  const policyHelpId = useId();

  return (
    <div className="grid gap-4">
      <label className="grid gap-2 text-sm font-semibold">
        {t("projects.name")}
        <input className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-transparent px-3 py-2 font-normal" maxLength={120} onChange={(event) => setName(event.target.value)} value={name} />
      </label>
      <label className="grid gap-2 text-sm font-semibold">
        {t("projects.description")}
        <textarea className="min-h-20 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-transparent px-3 py-2 font-normal" maxLength={2000} onChange={(event) => setDescription(event.target.value)} value={description} />
      </label>
      {!hideInstructions ? (
        <label className="grid gap-2 text-sm font-semibold">
          {t("projects.instructions")}
          <textarea className="min-h-28 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-transparent px-3 py-2 font-normal" maxLength={12000} onChange={(event) => setInstructions(event.target.value)} value={instructions} />
        </label>
      ) : null}
      <div className="mt-2 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-4">
        <label className="grid gap-2 text-sm font-semibold">
          {t("projects.policy")}
          <select aria-describedby={policyHelpId} className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 font-normal" onChange={(event) => setPolicy(event.target.value as ProjectPolicy)} value={policy}>
            <option value="local_only">{t("projects.policy_local")}</option>
            <option value="ask_before_cloud">{t("projects.policy_ask")}</option>
            <option value="allow_configured_cloud">{t("projects.policy_cloud")}</option>
          </select>
        </label>
        <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]" id={policyHelpId}>
          {t("projects.policy_help")}
        </p>
      </div>
    </div>
  );
}

function SourceRow({ projectId, source, setError, setSources, t }: { projectId: string; source: ProjectSource; setError: (value: string) => void; setSources: (value: ProjectSource[]) => void; t: TranslateFn }) {
  const unhealthy = source.grantState !== "active" || source.indexingState === "failed";
  const act = async (operation: () => Promise<unknown>) => {
    try { await operation(); setError(""); }
    catch { setError(t("projects.source_recovery")); }
    finally { setSources(await projectApi.sources(projectId)); }
  };
  const status = unhealthy
    ? t("projects.source_recovery")
    : source.fileCount === 0
      ? t("projects.source_empty")
      : t("projects.source_health", { state: projectSourceStateLabel(t, source.indexingState), count: source.fileCount });
  return <article className="flex items-start justify-between gap-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-4"><div className="min-w-0"><p className="truncate text-sm font-semibold">{source.canonicalPath}</p><p className={`mt-1 text-xs ${unhealthy ? "text-[var(--warning)]" : "text-[var(--foreground-muted)]"}`}>{status}</p></div><div className="flex shrink-0 gap-2"><button className="rounded border px-2 py-1 text-xs" onClick={() => void act(() => projectApi.refreshSource(projectId, source.sourceId))} type="button">{t("common.refresh")}</button><button className="rounded border px-2 py-1 text-xs" onClick={() => void act(() => projectApi.revokeSource(projectId, source.sourceId))} type="button">{t("common.remove")}</button></div></article>;
}
