export type ProjectCloudConsentChoice = "approve_once" | "always" | "cancel";

export type ProjectCloudConsentAttention = {
  sessionId: string;
  destination: string;
};

type ProjectCloudConsentCardProps = {
  attention: ProjectCloudConsentAttention;
  onChoice: (choice: ProjectCloudConsentChoice) => void;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

export function ProjectCloudConsentCard({
  attention,
  onChoice,
  t,
}: ProjectCloudConsentCardProps) {
  return (
    <section
      aria-labelledby="project-cloud-consent-title"
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--warning)] bg-[var(--warning-background)] px-5 py-4 text-[var(--foreground)]"
      role="alert"
    >
      <h3 className="text-sm font-semibold" id="project-cloud-consent-title">
        {t("chat.project_cloud_consent.title")}
      </h3>
      <p className="mt-1 text-sm leading-6">
        {t("chat.project_cloud_consent.body", {
          destination: attention.destination,
        })}
      </p>
      <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
        {t("chat.project_cloud_consent.disclosure")}
      </p>
      <div className="mt-4 flex flex-wrap gap-2">
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
          onClick={() => onChoice("approve_once")}
          type="button"
        >
          {t("chat.project_cloud_consent.approve_once")}
        </button>
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium transition-colors hover:bg-[var(--fill-hover)]"
          onClick={() => onChoice("always")}
          type="button"
        >
          {t("chat.project_cloud_consent.always")}
        </button>
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] px-3 py-2 text-sm font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
          onClick={() => onChoice("cancel")}
          type="button"
        >
          {t("chat.project_cloud_consent.cancel")}
        </button>
      </div>
    </section>
  );
}
