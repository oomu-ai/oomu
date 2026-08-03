import type { ChatSession } from "@/lib/chatSessions";
import { useI18n } from "@/context/I18nContext";

type ChatSessionDeleteToastProps = {
  session: ChatSession | null;
  onUndo: () => void;
};

export function ChatSessionDeleteToast({ session, onUndo }: ChatSessionDeleteToastProps) {
  const { t } = useI18n();
  if (!session) return null;
  return (
    <div
      aria-atomic="true"
      aria-live="polite"
      className="fixed top-16 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 rounded-full border border-[var(--border-soft)] bg-[var(--background)] py-1.5 pl-4 pr-1.5 shadow-lg"
      data-testid="chat-session-delete-toast"
      role="status"
    >
      <span className="text-sm text-[var(--foreground)]">
        {t("chat.session_deleted", { title: session.title })}
      </span>
      <button
        className="rounded-full px-3 py-1.5 text-sm font-medium text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)]"
        onClick={onUndo}
        type="button"
      >
        {t("common.undo")}
      </button>
    </div>
  );
}
