export type ApplicationUpdateStatus =
  | "checking"
  | "up_to_date"
  | "update_available"
  | "downloading"
  | "verifying"
  | "ready_to_restart"
  | "failed";

export type ApplicationUpdateCheckResult = {
  status: "checking" | "up_to_date" | "update_available" | "failed";
  origin: "automatic" | "manual";
  currentVersion: string;
  availableVersion?: string;
  notes?: string;
  fullNotesAvailable?: boolean;
  publicCode?: string;
  retryable?: boolean;
};

export type ApplicationUpdateInstallEvent = {
  status: "downloading" | "verifying" | "ready_to_restart" | "failed";
  downloadedBytes?: number;
  totalBytes?: number;
  publicCode?: string;
  retryable?: boolean;
};

export type ApplicationUpdateView = {
  status: ApplicationUpdateStatus;
  currentVersion: string;
  availableVersion?: string;
  notes?: string;
  fullNotesAvailable?: boolean;
  downloadedBytes?: number;
  totalBytes?: number;
  publicCode?: string;
  retryable?: boolean;
};

export function progressPercent(downloaded?: number, total?: number) {
  if (!downloaded || !total || total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((downloaded / total) * 100)));
}

export function formatUpdateBytes(bytes?: number) {
  if (!bytes || bytes <= 0) return "0 MB";
  const megabytes = bytes / (1024 * 1024);
  return `${megabytes >= 100 ? Math.round(megabytes) : megabytes.toFixed(1)} MB`;
}

export function checkResultView(result: ApplicationUpdateCheckResult): ApplicationUpdateView {
  return {
    status: result.status,
    currentVersion: result.currentVersion,
    availableVersion: result.availableVersion,
    notes: result.notes,
    fullNotesAvailable: result.fullNotesAvailable,
    publicCode: result.publicCode,
    retryable: result.retryable,
  };
}

