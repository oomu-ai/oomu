"use client";

import { useI18n } from "@/context/I18nContext";
import type { P0EventEnvelope } from "@/lib/p0Contracts";

const evidenceTones: Record<string, string> = {
  model_assertion: "bg-[var(--accent-background)]",
  observed_result: "bg-[var(--accent-background)]",
  executed_mutation: "bg-[var(--warning-background)]",
  verified_postcondition: "bg-[var(--success-background)]",
  signed_artifact: "bg-[var(--success-background)]",
};

const knownEvidence = new Set(Object.keys(evidenceTones));

export function EvidenceTimeline({ events, emptyLabel }: { events: P0EventEnvelope[]; emptyLabel: string }) {
  const { t } = useI18n();
  if (events.length === 0) return <p className="mt-2 text-sm text-[var(--foreground-muted)]">{emptyLabel}</p>;
  return <ol className="mt-3 flex flex-col gap-3">{events.map((event, index) => {
    const evidenceClass = knownEvidence.has(event.evidenceClass) ? event.evidenceClass : "unknown";
    const tone = evidenceTones[evidenceClass] ?? "bg-[var(--accent-background)]";
    const postcondition = event.evidenceClass === "executed_mutation" ? events.slice(index + 1).find((candidate) => candidate.evidenceClass === "verified_postcondition") : null;
    const connector = connectorEvidence(event);
    return <li className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-3" key={`${event.taskRunId}-${event.sequence}`}><div className="flex justify-between gap-3 text-xs"><span className="font-semibold">{connector ? connectorCapabilityLabel(connector.capability, t) : t(`evidence.classes.${evidenceClass}.label`)}</span><time className="text-[var(--foreground-muted)]">{new Date(event.timestamp).toLocaleString()}</time></div><div className={`mt-2 rounded px-2 py-2 text-xs ${tone}`}><p>{t(`evidence.classes.${evidenceClass}.detail`)}</p>{connector ? <p className="mt-2 font-medium">{connector.partial ? t("evidence_labels.partial_result") : t("evidence_labels.complete_result")}</p> : null}{postcondition ? <p className="mt-2 font-medium">{t("evidence.followup_confirmed")}</p> : event.evidenceClass === "executed_mutation" ? <p className="mt-2 font-semibold text-[var(--warning)]">{t("evidence.followup_pending")}</p> : null}</div><details className="mt-2 text-xs"><summary className="cursor-pointer text-[var(--foreground-muted)]">{t("evidence.details")}</summary><dl className="mt-2 grid grid-cols-[7rem_minmax(0,1fr)] gap-1 rounded bg-[var(--accent-background)] p-2"><dt className="text-[var(--foreground-muted)]">{t("evidence.sequence")}</dt><dd>{event.sequence}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence.recorded")}</dt><dd>{new Date(event.timestamp).toLocaleString()}</dd>{connector ? <ConnectorEvidenceDetails connector={connector} /> : null}</dl></details></li>;
  })}</ol>;
}

type ConnectorEvidence = {
  capability: string;
  citation: string;
  freshness: string;
  origin: string;
  partial: boolean;
  accountBound: boolean;
  tenantBound: boolean;
  postconditionRecorded: boolean;
};

function ConnectorEvidenceDetails({ connector }: { connector: ConnectorEvidence }) {
  const { t } = useI18n();
  return <><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.source")}</dt><dd className="break-all">{connector.citation}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.freshness")}</dt><dd>{t(`evidence_labels.freshness_${knownFreshness(connector.freshness)}`)}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.route")}</dt><dd>{t(`evidence_labels.route_${knownOrigin(connector.origin)}`)}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.account")}</dt><dd>{connector.accountBound ? t("evidence_labels.identity_confirmed") : t("evidence_labels.identity_unavailable")}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.organization")}</dt><dd>{connector.tenantBound ? t("evidence_labels.identity_confirmed") : t("evidence_labels.not_applicable")}</dd><dt className="text-[var(--foreground-muted)]">{t("evidence_labels.result_check")}</dt><dd>{connector.postconditionRecorded ? t("evidence_labels.postcondition_confirmed") : t("evidence_labels.read_only_result")}</dd></>;
}

function connectorEvidence(event: P0EventEnvelope): ConnectorEvidence | null {
  if (event.eventType !== "connector.tool.completed" || !isRecord(event.payload)) return null;
  const source = isRecord(event.payload.source) ? event.payload.source : null;
  if (typeof event.payload.capability !== "string" || !source || typeof source.citation !== "string" || typeof source.freshness !== "string" || typeof source.origin !== "string") return null;
  return {
    capability: event.payload.capability,
    citation: source.citation,
    freshness: source.freshness,
    origin: source.origin,
    partial: event.payload.partial === true,
    accountBound: typeof event.payload.accountBindingHash === "string",
    tenantBound: typeof event.payload.tenantBindingHash === "string",
    postconditionRecorded: event.payload.postcondition !== null && event.payload.postcondition !== undefined,
  };
}

const CONNECTOR_CAPABILITIES = new Set(["find_email", "read_email", "draft_email", "read_calendar", "draft_calendar_event", "find_personal_files", "read_personal_file", "save_personal_file", "find_team_site", "find_team_files", "read_team_file", "save_team_file", "list_chats", "find_chat_messages", "draft_chat_message"]);
function connectorCapabilityLabel(value: string, t: (key: string) => string) { return t(`evidence_labels.capability_${CONNECTOR_CAPABILITIES.has(value) ? value : "other"}`); }
function knownFreshness(value: string) { return ["live", "local_draft"].includes(value) ? value : "unknown"; }
function knownOrigin(value: string) {
  if (value === "microsoft_graph" || value === "microsoft_service" || value === "https://graph.microsoft.com") return "microsoft_graph";
  if (value === "google_workspace" || value.includes("googleapis.com")) return "google_workspace";
  if (value === "slack" || value === "https://slack.com") return "slack";
  if (value === "local" || value === "local_draft") return "local";
  return "other";
}
function isRecord(value: unknown): value is Record<string, unknown> { return Boolean(value) && typeof value === "object" && !Array.isArray(value); }
