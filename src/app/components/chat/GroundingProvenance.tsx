import { useI18n } from "@/context/I18nContext";
import type { PublicGroundingProvenance } from "./messageMetadata";

function formatAccessed(iso: string, locale: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function GroundingProvenance({
  sources,
}: {
  sources?: PublicGroundingProvenance[];
}) {
  const { language, t } = useI18n();
  if (!sources?.length) return null;

  return (
    <section
      aria-label={t("chat.grounding_sources.title")}
      className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background-subtle)] px-3 py-2.5"
    >
      <p className="text-xs font-semibold text-[var(--foreground)]">
        {t("chat.grounding_sources.title")}
      </p>
      <ul className="mt-2 space-y-2">
        {sources.map((source) => (
          <li className="min-w-0 text-xs leading-relaxed" key={`${source.url}:${source.accessedAtUtc}`}>
            <a
              className="block break-all font-medium text-[var(--accent)] underline decoration-transparent underline-offset-2 transition-colors hover:decoration-current"
              href={source.url}
              rel="noreferrer"
              target="_blank"
            >
              {source.url}
            </a>
            <time
              className="block text-[var(--foreground-muted)]"
              dateTime={source.accessedAtUtc}
            >
              {t("chat.grounding_sources.accessed_at", {
                time: formatAccessed(source.accessedAtUtc, language),
              })}
            </time>
          </li>
        ))}
      </ul>
    </section>
  );
}
