"use client";

import { useId } from "react";
import { useI18n } from "@/context/I18nContext";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useApprovalDialogTurn } from "@/context/ApprovalContext";

type McpConfirmationModalProps = {
  argumentsValue: unknown;
  argumentsLabel?: string;
  approveLabel?: string;
  canApprove?: boolean;
  cancelLabel?: string;
  isOpen: boolean;
  isResolving?: boolean;
  serverName: string;
  serverLabel?: string;
  scopeNotice?: string;
  title?: string;
  toolName: string;
  toolLabel?: string;
  onApprove: () => void;
  onCancel: () => void;
};

export function McpConfirmationModal({
  argumentsValue,
  argumentsLabel,
  approveLabel,
  canApprove = true,
  cancelLabel,
  isOpen,
  isResolving = false,
  onApprove,
  onCancel,
  serverName,
  serverLabel,
  scopeNotice,
  title,
  toolName,
  toolLabel,
}: McpConfirmationModalProps) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(isOpen, `mcp-confirmation-${dialogId}`);
  if (!isOpen || !hasDialogTurn) {
    return null;
  }

  const resolvedArgumentsLabel = argumentsLabel ?? t("mcp_confirmation.arguments");
  const resolvedApproveLabel = approveLabel ?? t("mcp_confirmation.approve");
  const resolvedCancelLabel = cancelLabel ?? t("mcp_confirmation.deny");
  const resolvedServerLabel = serverLabel ?? t("mcp_confirmation.server");
  const resolvedTitle = title ?? t("mcp_confirmation.title");
  const resolvedToolLabel = toolLabel ?? t("mcp_confirmation.tool");
  const safeArgumentRows = safePermissionArgumentRows(argumentsValue);
  const safeServerName = safeIdentifier(serverName) ?? resolvedServerLabel;
  const safeToolName = safeIdentifier(toolName) ?? resolvedToolLabel;

  return (
    <ApprovalDialogFrame
      description={t("mcp_confirmation.help")}
      eyebrow={t("mcp_confirmation.paused")}
      footer={<>
        <button className="cursor-pointer rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50" data-approval-initial-focus disabled={isResolving} onClick={onCancel} type="button">
          {resolvedCancelLabel}
        </button>
        <button aria-busy={isResolving} className="cursor-pointer rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50" disabled={isResolving || !canApprove} onClick={onApprove} type="button">
          {resolvedApproveLabel}
        </button>
      </>}
      maxWidthClassName="max-w-2xl"
      onDismiss={() => { if (!isResolving) onCancel(); }}
      title={resolvedTitle}
    >
          <div className="mt-5 space-y-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4 text-xs text-[var(--foreground-muted)]">
            <dl className="grid gap-3 sm:grid-cols-2">
              <div>
                <dt className="font-medium">{resolvedToolLabel}</dt>
                <dd className="mt-1 break-words font-semibold text-[var(--foreground)]">
                  {safeToolName}
                </dd>
              </div>
              <div>
                <dt className="font-medium">{resolvedServerLabel}</dt>
                <dd className="mt-1 break-words font-semibold text-[var(--foreground)]">
                  {safeServerName}
                </dd>
              </div>
            </dl>
            <div>
              <div className="mb-2 font-medium">{resolvedArgumentsLabel}</div>
              <div className="rounded-[var(--radius-sm)] bg-[var(--background)] p-3 text-[11px] leading-5 text-[var(--foreground)]">
                <ArgumentDetails rows={safeArgumentRows} />
              </div>
            </div>
          </div>
          {scopeNotice ? (
            <p className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-sm text-[var(--foreground-muted)]">
              {scopeNotice}
            </p>
          ) : null}
          {!canApprove ? <p className="mt-4 rounded-[var(--radius-sm)] bg-[var(--warning-background)] p-3 text-sm font-medium text-[var(--warning)]" role="alert">{t("permissions.unverified_action")}</p> : null}
    </ApprovalDialogFrame>
  );
}

type SafeArgumentRow = {
  labelKey?:
    | "mcp_confirmation.destination"
    | "mcp_confirmation.location"
    | "mcp_confirmation.purpose";
  value: string;
};

function ArgumentDetails({ rows }: { rows: SafeArgumentRow[] }) {
  const { t } = useI18n();
  if (rows.length === 0) {
    return <span className="text-[var(--foreground-muted)]">{t("mcp_confirmation.no_arguments")}</span>;
  }
  return <ul className="grid gap-2">{rows.map((row, index) => <li className="rounded border border-[var(--border-soft)] p-2" key={`${row.labelKey ?? "summary"}-${index}`}>
    {row.labelKey ? <span className="block font-medium text-[var(--foreground-muted)]">{t(row.labelKey)}</span> : null}
    <span className="mt-0.5 block break-words">{row.value}</span>
  </li>)}</ul>;
}

const PURPOSE_FIELDS = ["purpose", "reason", "capabilityReason", "capability_reason"];
const DESTINATION_FIELDS = ["destination", "channel", "conversation", "recipient", "recipients", "to"];
const LOCATION_FIELDS = [
  "path", "targetPath", "target_path", "destinationPath", "destination_path",
  "filePath", "file_path", "folder", "workingDirectory", "working_directory",
];

function safePermissionArgumentRows(value: unknown): SafeArgumentRow[] {
  if (Array.isArray(value)) {
    return value
      .flatMap((item) => safeScalarValues(item).map((entry) => ({ value: entry })))
      .slice(0, 4);
  }
  if (isPlainRecord(value)) {
    const nested = isPlainRecord(value.arguments) ? value.arguments : null;
    const rows: SafeArgumentRow[] = [];
    addKnownRow(rows, value, PURPOSE_FIELDS, "mcp_confirmation.purpose", "plain");
    if (nested && rows.length === 0) {
      addKnownRow(rows, nested, PURPOSE_FIELDS, "mcp_confirmation.purpose", "plain");
    }
    const beforeDestination = rows.length;
    addKnownRow(rows, nested ?? value, DESTINATION_FIELDS, "mcp_confirmation.destination", "destination");
    if (nested && rows.length === beforeDestination) {
      addKnownRow(rows, value, DESTINATION_FIELDS, "mcp_confirmation.destination", "destination");
    }
    const beforeLocation = rows.length;
    addKnownRow(rows, nested ?? value, LOCATION_FIELDS, "mcp_confirmation.location", "location");
    if (nested && rows.length === beforeLocation) {
      addKnownRow(rows, value, LOCATION_FIELDS, "mcp_confirmation.location", "location");
    }
    return rows.slice(0, 4);
  }
  return safeScalarValues(value).map((entry) => ({ value: entry })).slice(0, 4);
}

function addKnownRow(
  rows: SafeArgumentRow[],
  source: Record<string, unknown>,
  fields: string[],
  labelKey: NonNullable<SafeArgumentRow["labelKey"]>,
  kind: "plain" | "destination" | "location",
) {
  for (const field of fields) {
    if (!(field in source)) continue;
    const values = safeScalarValues(source[field], kind);
    if (values.length) {
      rows.push({ labelKey, value: values.join(", ").slice(0, 160) });
      return;
    }
  }
}

function safeScalarValues(
  value: unknown,
  kind: "plain" | "destination" | "location" = "plain",
) {
  const values = Array.isArray(value) ? value.slice(0, 3) : [value];
  return values.flatMap((entry) => {
    if (typeof entry !== "string" && typeof entry !== "number" && typeof entry !== "boolean") return [];
    const raw = String(entry);
    const safe = kind === "location"
      ? friendlyLocation(raw)
      : kind === "destination"
        ? friendlyDestination(raw)
        : sanitizePlainText(raw);
    return safe ? [safe] : [];
  });
}

function friendlyLocation(value: string): string | null {
  if (/^https?:\/\//i.test(value)) return friendlyDestination(value);
  const basename = value.replace(/\\/g, "/").split("/").filter(Boolean).at(-1);
  return sanitizePlainText(basename ?? value, 80);
}

function friendlyDestination(value: string): string | null {
  try {
    const url = new URL(value);
    if (["http:", "https:"].includes(url.protocol)) {
      return sanitizePlainText(url.hostname.replace(/^www\./i, ""), 100);
    }
  } catch {
    // Non-URL destinations continue through the plain-text safety filter.
  }
  if (/[\\/]/.test(value) && !value.includes("@")) return friendlyLocation(value);
  return sanitizePlainText(value, 100);
}

function safeIdentifier(value: string) {
  if (/https?:\/\/|[\\/]/i.test(value)) return null;
  return sanitizePlainText(
    value
      .replace(/\b(?:wfi|wf)-[A-Za-z0-9:_-]+/g, "")
      .replace(/[_:./-]+/g, " ")
      .replace(/\s+/g, " ")
      .trim(),
    80,
  );
}

function sanitizePlainText(value: string, maxLength = 120) {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (/https?:\/\//i.test(trimmed) || /(?:^|\s)(?:\.\.\/?|~\/|\/Users\/|\/private\/|\/var\/|[A-Za-z]:\\)/.test(trimmed)) return null;
  if (/(?:```|~~~|`[^`]*`|<[^>]+>)/.test(trimmed)) return null;
  if (/\b(?:bearer|api[ _-]?key|authorization|cookie|credential|password|secret|signature|token)\b/i.test(trimmed)) return null;
  if (/\b[a-f0-9]{24,}\b/i.test(trimmed) || /^[A-Za-z0-9+/_-]{32,}={0,2}$/.test(trimmed)) return null;
  if (/(?:&&|\|\||\$\(|(?:^|\s)--[a-z]|(?:^|\s)(?:sudo|curl|wget|rm|chmod|chown|bash|zsh|python|node|osascript|powershell)\s)/i.test(trimmed)) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if ((parsed && typeof parsed === "object") || trimmed.startsWith('"')) return null;
  } catch {
    // Ordinary prose is not JSON.
  }
  const cleaned = trimmed
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])\s+/gm, "")
    .replace(/[\u0000-\u001F\u007F{}]/g, " ")
    .replace(/[*_~]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return null;
  return cleaned.length > maxLength
    ? `${cleaned.slice(0, maxLength - 1).trimEnd()}…`
    : cleaned;
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
