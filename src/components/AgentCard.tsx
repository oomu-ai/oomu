/* eslint-disable @next/next/no-img-element */

import { useI18n } from "@/context/I18nContext";

type AgentCardProps = {
  name: string;
  description: string;
  modBadges?: Array<{ id: string; name: string }>;
  isFavorite?: boolean;
  canFavorite?: boolean;
  onOpen?: () => void;
  onToggleFavorite?: () => void;
};

function RobotIcon() {
  return (
    <svg aria-hidden="true" className="h-12 w-12" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <rect height="12" width="16" x="4" y="8" rx="1" />
      <path d="M9 13h.01M15 13h.01M12 21v-1M12 8V5m0 0a2 2 0 1 1 0-4 2 2 0 0 1 0 4Z" />
    </svg>
  );
}

export default function AgentCard({
  name,
  description,
  modBadges = [],
  image,
  isFavorite = false,
  canFavorite = false,
  onOpen,
  onToggleFavorite,
}: AgentCardProps & { image?: string | null }) {
  const { t } = useI18n();
  return (
    <div
      aria-label={t("agents.open_named", { name })}
      className="flex items-start justify-center cursor-pointer rounded-[var(--radius-lg)]"
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen?.();
        }
      }}
      role="button"
      tabIndex={0}
    >
      <div className="w-full max-w-[15.7rem] aspect-[3/3.25] flex flex-col">
        <div className="flex-1 overflow-hidden rounded-[var(--radius-lg)] border border-[var(--border-strong)] bg-[var(--background)] flex flex-col">
          <div className="relative h-[1.65rem] border-b border-[var(--border-soft)] bg-[var(--accent-background)] flex items-center justify-center px-8">
            <span className="text-sm font-semibold text-[var(--foreground)] tracking-tight truncate">
              {name}
            </span>
            {canFavorite && (
              <button
                aria-label={isFavorite ? t("agents.remove_favorite", { name }) : t("agents.add_favorite", { name })}
                className="absolute right-1 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center text-[var(--foreground-subtle)] transition-colors hover:text-[var(--accent)]"
                onClick={(event) => {
                  event.stopPropagation();
                  onToggleFavorite?.();
                }}
                type="button"
              >
                <svg
                  aria-hidden="true"
                  className={`h-4 w-4 ${isFavorite ? "text-[var(--accent)]" : ""}`}
                  fill={isFavorite ? "currentColor" : "none"}
                  stroke="currentColor"
                  strokeLinejoin="round"
                  strokeWidth="1.8"
                  viewBox="0 0 24 24"
                >
                  <path d="m12 3.5 2.6 5.4 5.9.8-4.3 4.2 1 5.9-5.2-2.8-5.2 2.8 1-5.9-4.3-4.2 5.9-.8L12 3.5Z" />
                </svg>
              </button>
            )}
          </div>
          <div className="h-[6.6rem] border-b border-[var(--border-soft)] bg-[var(--accent-background)] flex items-center justify-center">
            <div className="flex h-[4.4rem] w-[4.4rem] items-center justify-center rounded-full border-2 border-[var(--border-soft)] bg-[var(--background)] text-[var(--foreground)] overflow-hidden">
              {image ? (
                <img src={image} alt={name} className="w-full h-full object-cover" />
              ) : (
                <RobotIcon />
              )}
            </div>
          </div>

          <div className="flex flex-col px-[1.1rem] py-[1.35rem] flex-1">
            <p className="text-sm font-medium leading-relaxed text-[var(--foreground)] tracking-tight line-clamp-3">
              {description}
            </p>
            {modBadges.length > 0 ? (
              <div className="mt-auto flex flex-wrap gap-1.5 pt-3">
                {modBadges.slice(0, 3).map((mod) => (
                  <span
                    className="max-w-full truncate rounded-full border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-0.5 text-[10px] font-semibold text-[var(--foreground-muted)]"
                    key={mod.id}
                    title={mod.name}
                  >
                    {mod.name}
                  </span>
                ))}
                {modBadges.length > 3 ? (
                  <span className="rounded-full border border-[var(--border-soft)] bg-[var(--background)] px-2 py-0.5 text-[10px] font-semibold text-[var(--foreground-subtle)]">
                    +{modBadges.length - 3}
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
