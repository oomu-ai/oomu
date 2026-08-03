type ProjectChatScopeBannerProps = {
  onStartGlobalChat: () => void;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

export function ProjectChatScopeBanner({
  onStartGlobalChat,
  t,
}: ProjectChatScopeBannerProps) {
  return (
    <section
      aria-label={t("chat.project_scope.label")}
      className="mx-2 mb-2 rounded-[var(--radius-md)] border border-[var(--accent)]/20 bg-[var(--accent-background)] px-3 py-2.5"
    >
      <p className="text-xs font-semibold text-[var(--foreground)]">
        {t("chat.project_scope.active")}
      </p>
      <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
        {t("chat.project_scope.help")}
      </p>
      <button
        className="mt-2 text-xs font-semibold text-[var(--accent)] transition-colors hover:text-[var(--accent-hover)]"
        onClick={onStartGlobalChat}
        type="button"
      >
        {t("chat.project_scope.start_global")}
      </button>
    </section>
  );
}
