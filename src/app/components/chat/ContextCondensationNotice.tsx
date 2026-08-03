import { useI18n } from "@/context/I18nContext";
import type { ChatMessageMetadata } from "./messageMetadata";

export function ContextCondensationNotice({ metadata }: { metadata: ChatMessageMetadata }) {
  const { t } = useI18n();
  if (!metadata.contextCondensed) return null;
  return (
    <p className="mt-3 text-xs leading-relaxed text-[var(--foreground-muted)]" role="status">
      {t("chat.context_condensation.notice")}
    </p>
  );
}
