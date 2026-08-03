type PersistenceLoadNoticeProps = {
  message: string;
  retryLabel: string;
  retrying: boolean;
  onRetry: () => void;
};

export function PersistenceLoadNotice({
  message,
  onRetry,
  retryLabel,
  retrying,
}: PersistenceLoadNoticeProps) {
  return (
    <div
      className="mx-4 mt-4 flex items-center justify-between gap-4 rounded-[var(--radius-md)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-4 py-3 text-sm"
      role="alert"
    >
      <p className="text-[var(--foreground)]">{message}</p>
      <button
        className="shrink-0 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-60"
        disabled={retrying}
        onClick={onRetry}
        type="button"
      >
        {retryLabel}
      </button>
    </div>
  );
}
