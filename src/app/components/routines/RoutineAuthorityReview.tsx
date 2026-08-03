import { useId } from "react";
import type { ChannelPlatform } from "./channelReadiness";
import type { RoutineTranslate } from "./routineLabels";
import type { WorkflowReviewCapabilities } from "./workflowReviewCapabilities";

export function RoutineAuthorityReview({
  deliveryDestination,
  deliveryPlatform,
  disabled,
  onReviewWorkflow,
  projectName,
  reviewCapabilities,
  t,
  workflowName,
}: {
  deliveryDestination: string;
  deliveryPlatform: ChannelPlatform | "";
  disabled: boolean;
  onReviewWorkflow: () => void;
  projectName: string;
  reviewCapabilities: WorkflowReviewCapabilities;
  t: RoutineTranslate;
  workflowName: string;
}) {
  const titleId = useId();
  const deliveryReview = deliveryPlatform
    ? deliveryDestination
      ? t("routines.authority_delivery_channel", {
          destination: deliveryDestination,
          platform: t(`routines.delivery_${deliveryPlatform}`),
        })
      : t("routines.authority_delivery_pending")
    : t("routines.authority_delivery_local");
  const exactApprovalKeys = [
    reviewCapabilities.calendarCreate && reviewCapabilities.emailSend
      ? "routines.authority_exact_approval"
      : reviewCapabilities.calendarCreate
        ? "routines.authority_calendar_approval"
        : reviewCapabilities.emailSend
          ? "routines.authority_email_approval"
          : null,
    reviewCapabilities.emailDraft
      ? "routines.authority_email_draft_approval"
      : null,
  ].filter((key): key is string => Boolean(key));

  return (
    <section
      aria-labelledby={titleId}
      className="mt-7 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--fill-hover)] p-5"
    >
      <h3 className="text-base font-semibold" id={titleId}>
        {t(
          reviewCapabilities.status === "ready"
            ? "routines.authority_title"
            : "routines.authority_unavailable_title",
        )}
      </h3>
      {reviewCapabilities.status === "unavailable" ? (
        <div className="mt-2">
          <p className="text-sm text-[var(--foreground-muted)]">
            {t("routines.authority_unavailable_help")}
          </p>
          <button
            className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm font-semibold"
            disabled={disabled}
            onClick={onReviewWorkflow}
            type="button"
          >
            {t("routines.review_workflow")}
          </button>
        </div>
      ) : (
        <dl className="mt-4 grid gap-3 text-sm">
          <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
            <dt className="text-[var(--foreground-muted)]">
              {t("routines.authority_workflow")}
            </dt>
            <dd className="font-medium">{workflowName}</dd>
          </div>
          <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
            <dt className="text-[var(--foreground-muted)]">
              {t("routines.authority_project")}
            </dt>
            <dd className="font-medium">{projectName}</dd>
          </div>
          {reviewCapabilities.projectFileRead ||
          reviewCapabilities.projectFileWrite ? (
            <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
              <dt className="text-[var(--foreground-muted)]">
                {t("routines.authority_files")}
              </dt>
              <dd>
                {t(
                  reviewCapabilities.projectFileRead &&
                    reviewCapabilities.projectFileWrite
                    ? "routines.authority_files_scope"
                    : reviewCapabilities.projectFileRead
                      ? "routines.authority_files_read_scope"
                      : "routines.authority_files_write_scope",
                  { project: projectName },
                )}
              </dd>
            </div>
          ) : null}
          {reviewCapabilities.officialWeb ? (
            <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
              <dt className="text-[var(--foreground-muted)]">
                {t("routines.authority_web")}
              </dt>
              <dd>{t("routines.authority_web_scope")}</dd>
            </div>
          ) : null}
          {reviewCapabilities.calendarRead ? (
            <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
              <dt className="text-[var(--foreground-muted)]">
                {t("routines.authority_calendar")}
              </dt>
              <dd>{t("routines.authority_calendar_read_scope")}</dd>
            </div>
          ) : null}
          {reviewCapabilities.emailRead ? (
            <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
              <dt className="text-[var(--foreground-muted)]">
                {t("routines.authority_mail")}
              </dt>
              <dd>{t("routines.authority_mail_read_scope")}</dd>
            </div>
          ) : null}
          <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3">
            <dt className="text-[var(--foreground-muted)]">
              {t("routines.authority_delivery")}
            </dt>
            <dd className="break-words">{deliveryReview}</dd>
          </div>
        </dl>
      )}
      {reviewCapabilities.status === "ready" && exactApprovalKeys.length > 0 ? (
        <ul className="mt-4 grid gap-1 border-t border-[var(--border-soft)] pt-4 text-sm font-medium">
          {exactApprovalKeys.map((key) => (
            <li key={key}>{t(key)}</li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
