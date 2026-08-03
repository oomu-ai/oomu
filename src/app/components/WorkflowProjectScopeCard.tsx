type WorkflowProjectScopeCardProps = {
  projectId: string | null;
  projectName: string | null;
  onChooseProject: () => void;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

export function WorkflowProjectScopeCard({
  projectId,
  projectName,
  onChooseProject,
  t,
}: WorkflowProjectScopeCardProps) {
  return (
    <section
      aria-label={t("workflows.scope.label")}
      className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4 sm:flex-row sm:items-center sm:justify-between"
    >
      <div>
        <p className="text-sm font-semibold text-[var(--foreground)]">
          {projectName
            ? t("workflows.scope.project_title", { project: projectName })
            : projectId
              ? t("workflows.scope.bound_title")
              : t("workflows.scope.global_title")}
        </p>
        <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
          {projectName
            ? t("workflows.scope.project_help", { project: projectName })
            : projectId
              ? t("workflows.scope.bound_help")
              : t("workflows.scope.global_help")}
        </p>
      </div>
      {!projectId ? (
        <button
          className="shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm font-medium"
          onClick={onChooseProject}
          type="button"
        >
          {t("workflows.scope.choose_project")}
        </button>
      ) : null}
    </section>
  );
}
