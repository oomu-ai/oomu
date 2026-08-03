export type PrivateEgressConsentChoice = "send_once" | "keep_private";

export type PrivateEgressConsentAttention = {
  sessionId: string;
  challengeId: string;
  destination: string;
  sourceNames: string[];
};

type PrivateEgressConsentCardProps = {
  attention: PrivateEgressConsentAttention;
  onChoice: (choice: PrivateEgressConsentChoice) => void;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

export function PrivateEgressConsentCard({
  attention,
  onChoice,
  t,
}: PrivateEgressConsentCardProps) {
  const sources = attention.sourceNames.join(", ");

  return (
    <section
      aria-labelledby="private-egress-consent-title"
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--warning)] bg-[var(--warning-background)] px-5 py-4 text-[var(--foreground)]"
      role="alert"
    >
      <h3 className="text-sm font-semibold" id="private-egress-consent-title">
        {t("chat.private_egress_consent.title", {
          destination: attention.destination,
        })}
      </h3>
      <p className="mt-1 text-sm leading-6">
        {t("chat.private_egress_consent.body", {
          destination: attention.destination,
          sources,
        })}
      </p>
      <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
        {t("chat.private_egress_consent.disclosure")}
      </p>
      <div className="mt-4 flex flex-wrap gap-2">
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
          onClick={() => onChoice("send_once")}
          type="button"
        >
          {t("chat.private_egress_consent.send_once")}
        </button>
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium transition-colors hover:bg-[var(--fill-hover)]"
          onClick={() => onChoice("keep_private")}
          type="button"
        >
          {t("chat.private_egress_consent.keep_private")}
        </button>
      </div>
    </section>
  );
}
