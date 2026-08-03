"use client";

import { invoke } from "@/lib/invoke";
import { useState, useMemo } from "react";
import { useI18n } from "@/context/I18nContext";
import { DEFAULT_LOCAL_MODEL_ID, type ConfiguredProvider } from "@/lib/modelRegistry";

type ScannedAgentFile = {
  key: string;
  filename: string;
  relative_path: string;
  size_bytes: number;
  modified_at_ms?: number | null;
  group?: string;
  label: string;
  description: string;
  selected_by_default: boolean;
};

type ScanAgentDirectoryResponse = {
  success: boolean;
  directory_name: string;
  scan_token: string;
  files: ScannedAgentFile[];
};

type AgentImportDirectoryGrant = {
  grant_id: string;
  directory_name: string;
  expires_at_ms: number;
};

type LogImportRange = "all_history" | "last_30_days" | "last_10_days" | "none";

const JOURNAL_GROUP = "chronological_journals";

const LOG_IMPORT_RANGE_OPTIONS: Array<{ value: LogImportRange; labelKey: string }> = [
  { value: "all_history", labelKey: "import.log_range.all_history" },
  { value: "last_30_days", labelKey: "import.log_range.last_30_days" },
  { value: "last_10_days", labelKey: "import.log_range.last_10_days" },
  { value: "none", labelKey: "import.log_range.none" },
];

type AgentPersonalityTemplate = {
  id: string;
  name: string;
  description: string;
  instructions: string;
  attributes: string[];
  origin: "system" | "custom";
};

type ImportAgentScreenProps<AgentConfig> = {
  configuredProviders: ConfiguredProvider[];
  templateOptions: AgentPersonalityTemplate[];
  refreshTarget?: ImportedAgentRefreshTarget;
  onImportComplete: (agentConfig: AgentConfig) => void;
  onCancel: () => void;
};

export type ImportedAgentRefreshTarget = {
  id: string;
  name: string;
  description: string;
  providerId: string;
  modelId: string;
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function isJournalFile(file: ScannedAgentFile) {
  return file.group === JOURNAL_GROUP;
}

function formatJournalDate(modifiedAtMs?: number | null) {
  if (!modifiedAtMs) {
    return null;
  }

  const date = new Date(modifiedAtMs);
  if (Number.isNaN(date.getTime())) {
    return null;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(date);
}

function journalDisplayName(
  file: ScannedAgentFile,
  index: number,
  t: (key: string, variables?: Record<string, string | number>) => string,
) {
  const dateLabel = formatJournalDate(file.modified_at_ms);
  return dateLabel
    ? t("import.journal_from_date", { date: dateLabel })
    : t("import.journal_number", { index: index + 1 });
}

function formatBytes(bytes: number) {
  if (bytes === 0) return "0 Bytes";
  const units = ["Bytes", "KB", "MB"];
  const unit = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${parseFloat((bytes / Math.pow(1024, unit)).toFixed(1))} ${units[unit]}`;
}

function ImportHeader({
  onCancel,
  refreshTarget,
  t,
}: {
  onCancel: () => void;
  refreshTarget?: ImportedAgentRefreshTarget;
  t: (key: string, variables?: Record<string, string | number>) => string;
}) {
  return <div className="flex items-center justify-between border-b border-[var(--border-strong)] pb-4">
    <div>
      <h2 className="text-base font-bold tracking-tight text-[var(--foreground)]">
        {refreshTarget
          ? t("sprint_299.import_refresh.title", { name: refreshTarget.name })
          : t("import.title")}
      </h2>
      <p className="mt-1 text-xs text-[var(--foreground-muted)]">
        {t(refreshTarget ? "sprint_299.import_refresh.description" : "import.description")}
      </p>
    </div>
    <button className="border border-[var(--border-strong)] px-3 py-2 text-sm font-medium transition-colors hover:bg-[var(--accent-background)]" onClick={onCancel} type="button">
      {t("common.cancel")}
    </button>
  </div>;
}

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 20 20"
      className={`h-4 w-4 shrink-0 transition-transform ${expanded ? "rotate-180" : ""}`}
      fill="none"
    >
      <path
        d="M5 7.5L10 12.5L15 7.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function ImportAgentScreen<AgentConfig>({
  configuredProviders,
  templateOptions,
  refreshTarget,
  onImportComplete,
  onCancel,
}: ImportAgentScreenProps<AgentConfig>) {
  const { t } = useI18n();
  const [directoryGrantId, setDirectoryGrantId] = useState("");
  const [directoryName, setDirectoryName] = useState("");
  const [isScanning, setIsScanning] = useState(false);
  const [isBrowsing, setIsBrowsing] = useState(false);
  const [scanResponse, setScanResponse] = useState<ScanAgentDirectoryResponse | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);

  // Form Fields
  const [agentName, setAgentName] = useState(refreshTarget?.name ?? "");
  const [agentDescription, setAgentDescription] = useState(refreshTarget?.description ?? "");
  const [selectedProviderId, setSelectedProviderId] = useState(refreshTarget?.providerId ?? "");
  const [selectedModelId, setSelectedModelId] = useState(refreshTarget?.modelId ?? "");
  const [selectedTemplate, setSelectedTemplate] = useState("everyday_agent");
  const [selectedFileKeys, setSelectedFileKeys] = useState<Record<string, boolean>>({});
  const [logImportRange, setLogImportRange] = useState<LogImportRange>("all_history");
  const [journalsExpanded, setJournalsExpanded] = useState(true);
  const [isImporting, setIsImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  // Map of selected model configuration
  const providerOptions = useMemo(() => {
    return configuredProviders.map((p) => ({
      id: p.id,
      name: p.providerName,
      providerId: p.providerId,
      customModelIds: p.customModelIds
        ? p.customModelIds.split(",").map((s: string) => s.trim()).filter(Boolean)
        : [],
    }));
  }, [configuredProviders]);

  const activeProvider = useMemo(() => {
    return providerOptions.find((p) => p.id === selectedProviderId);
  }, [providerOptions, selectedProviderId]);

  const scanDirectoryGrant = async (
    targetGrantId: string,
    range: LogImportRange = logImportRange,
    preserveSelections = false
  ) => {
    if (!targetGrantId.trim()) return;
    setIsScanning(true);
    setScanError(null);
    setScanResponse(null);

    try {
      const response = await invoke<ScanAgentDirectoryResponse>("scan_agent_import_directory", {
        grantId: targetGrantId.trim(),
        logImportRange: range,
      });
      setScanResponse(response);

      // Auto-prefill Agent Name from directory name
      if (!refreshTarget) {
        setAgentName(response.directory_name || "");
        setAgentDescription(
          t("import.imported_description", { name: response.directory_name }),
        );
      }

      // Select files by default
      setSelectedFileKeys((previous) => {
        const defaultSelections: Record<string, boolean> = {};
        response.files.forEach((file) => {
          defaultSelections[file.key] =
            preserveSelections && file.key in previous
              ? previous[file.key]
              : file.selected_by_default;
        });
        return defaultSelections;
      });
      setJournalsExpanded(response.files.some(isJournalFile) || range !== "all_history");

      // Select first provider by default if available
      if (!refreshTarget && providerOptions.length > 0) {
        const firstProv = providerOptions[0];
        setSelectedProviderId(firstProv.id);
        if (firstProv.customModelIds.length > 0) {
          setSelectedModelId(firstProv.customModelIds[0]);
        } else {
          setSelectedModelId(DEFAULT_LOCAL_MODEL_ID);
        }
      }
    } catch (error: unknown) {
      setScanError(errorMessage(error));
    } finally {
      setIsScanning(false);
    }
  };

  const handleScanDirectory = async () => {
    await scanDirectoryGrant(directoryGrantId, logImportRange);
  };

  const handleBrowseDirectory = async () => {
    setIsBrowsing(true);
    setScanError(null);
    try {
      const selected = await invoke<AgentImportDirectoryGrant | null>(
        "choose_agent_import_directory",
      );
      if (selected) {
        setDirectoryGrantId(selected.grant_id);
        setDirectoryName(selected.directory_name);
        await scanDirectoryGrant(selected.grant_id, logImportRange);
      }
    } catch (error: unknown) {
      setScanError(errorMessage(error));
    } finally {
      setIsBrowsing(false);
    }
  };

  const handleFileCheckboxChange = (key: string) => {
    setSelectedFileKeys((prev) => ({
      ...prev,
      [key]: !prev[key],
    }));
  };

  const handleLogImportRangeChange = async (range: LogImportRange) => {
    setLogImportRange(range);
    if (scanResponse && directoryGrantId.trim()) {
      await scanDirectoryGrant(directoryGrantId, range, true);
    }
  };

  const blueprintFiles = useMemo(() => {
    return (scanResponse?.files ?? []).filter((file) => !isJournalFile(file));
  }, [scanResponse]);

  const journalFiles = useMemo(() => {
    return (scanResponse?.files ?? []).filter(isJournalFile);
  }, [scanResponse]);

  const selectedFileCount = useMemo(() => {
    return (scanResponse?.files ?? []).filter((file) => selectedFileKeys[file.key]).length;
  }, [scanResponse, selectedFileKeys]);

  const allFilesSelected = useMemo(() => {
    const files = scanResponse?.files ?? [];
    return files.length > 0 && files.every((file) => selectedFileKeys[file.key]);
  }, [scanResponse, selectedFileKeys]);

  const selectedJournalCount = useMemo(() => {
    return journalFiles.filter((file) => selectedFileKeys[file.key]).length;
  }, [journalFiles, selectedFileKeys]);

  const showJournalPanel = journalFiles.length > 0 || logImportRange !== "all_history";

  const handleToggleAllFiles = () => {
    if (!scanResponse) return;
    const nextSelected = !allFilesSelected;
    const nextSelections: Record<string, boolean> = {};
    scanResponse.files.forEach((file) => {
      nextSelections[file.key] = nextSelected;
    });
    setSelectedFileKeys(nextSelections);
  };

  const handleProviderChange = (providerId: string) => {
    setSelectedProviderId(providerId);
    const prov = providerOptions.find((p) => p.id === providerId);
    if (prov && prov.customModelIds.length > 0) {
      setSelectedModelId(prov.customModelIds[0]);
    } else {
      setSelectedModelId("");
    }
  };

  const handleImportAgent = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!agentName.trim()) return;
    setIsImporting(true);
    setImportError(null);

    const keysToImport = Object.keys(selectedFileKeys).filter((k) => selectedFileKeys[k]);

    try {
      const result = await invoke<AgentConfig>("execute_agent_import", {
        request: {
          grantId: directoryGrantId.trim(),
          scanToken: scanResponse?.scan_token ?? "",
          keysToImport,
          agentName: agentName.trim(),
          agentDescription: agentDescription.trim(),
          modelId: selectedModelId || DEFAULT_LOCAL_MODEL_ID,
          providerId: selectedProviderId || "local_model",
          personalityTemplate: selectedTemplate,
          targetAgentId: refreshTarget?.id,
        },
      });
      onImportComplete(result);
    } catch (error: unknown) {
      setImportError(errorMessage(error));
    } finally {
      setIsImporting(false);
    }
  };

  return (
    <div className="flex flex-col gap-6 max-w-[64rem] mx-auto w-full p-4">
      <ImportHeader onCancel={onCancel} refreshTarget={refreshTarget} t={t} />

      {/* Directory Scanner Row */}
      <section className="flex flex-col gap-4 rounded-[var(--radius-md)] border border-[var(--border-strong)] p-6 bg-[var(--background)]">
        <h3 className="text-sm font-semibold text-[var(--foreground)]">
          {t("import.step_folder")}
        </h3>
        <p className="text-sm text-[var(--foreground-muted)] leading-relaxed">
          {t("import.step_folder_desc")}
        </p>
        <div className="flex gap-3">
          <div className="flex-1 flex overflow-hidden rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] transition-colors focus-within:bg-[var(--accent-background)]">
            <input
              type="text"
              className="flex-1 bg-transparent px-4 py-2.5 text-sm font-medium text-[var(--foreground)] outline-none"
              placeholder={t("import.step_folder_placeholder")}
              value={directoryName}
              readOnly
              disabled={isScanning || isImporting || isBrowsing}
            />
            <button
              type="button"
              onClick={handleBrowseDirectory}
              disabled={isScanning || isImporting || isBrowsing}
              className="border-l border-[var(--border-strong)] px-4 text-sm font-medium text-[var(--foreground-muted)] hover:bg-[var(--accent-background)] transition-colors disabled:opacity-50 shrink-0"
            >
              {isBrowsing ? t("common.choosing") : t("common.choose")}
            </button>
          </div>
          <button
            type="button"
            onClick={handleScanDirectory}
            disabled={isScanning || isImporting || isBrowsing || !directoryGrantId.trim()}
            className="bg-[var(--inverse-background)] text-[var(--inverse-foreground)] px-6 py-2.5 text-sm font-semibold hover:bg-[var(--accent-hover)] transition-colors disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
          >
            {isScanning ? t("import.reviewing_button") : t("import.review_button")}
          </button>
        </div>
        {scanError && (
          <div className="text-[var(--destructive)] text-xs font-semibold mt-2 border-l-2 border-[var(--destructive)] pl-3">
            {t("import.folder_error", { error: scanError })}
          </div>
        )}
      </section>

      {/* Scan Results & Import Configuration Form */}
      {scanResponse && (
        <form onSubmit={handleImportAgent} className="flex flex-col gap-6">
          <div className="grid gap-6 lg:grid-cols-[1.2fr_1fr]">
            {/* Scanned Files List */}
            <div className="rounded-[var(--radius-md)] border border-[var(--border-strong)] p-6 flex flex-col gap-4 bg-[var(--background)] h-fit">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-sm font-semibold text-[var(--foreground)]">
                    {t("import.step_files")}
                  </h3>
                  {scanResponse.files.length > 0 && (
                    <p className="mt-1 text-[11px] font-medium text-[var(--foreground-muted)]">
                      {t("import.selected_count", {
                        selected: selectedFileCount,
                        total: scanResponse.files.length,
                      })}
                    </p>
                  )}
                </div>
                <button
                  type="button"
                  onClick={handleToggleAllFiles}
                  disabled={scanResponse.files.length === 0}
                  className="shrink-0 border border-[var(--border-strong)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--accent-background)] disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {allFilesSelected ? t("import.clear_all") : t("import.select_all")}
                </button>
              </div>
              <p className="text-xs text-[var(--foreground-muted)] leading-relaxed mb-2">
                {t("import.files_desc")}
              </p>

              {scanResponse.files.length === 0 && !showJournalPanel ? (
                <div className="text-xs text-[var(--foreground-muted)] py-6 text-center rounded-[var(--radius-md)] border border-[var(--border-soft)]">
                  {t("import.no_files_found")}
                </div>
              ) : (
                <div className="flex flex-col gap-3">
                  {blueprintFiles.map((file) => (
                    <label
                      key={file.key}
                      className="flex items-start gap-3 rounded-[var(--radius-md)] border border-[var(--border-strong)] p-3 hover:bg-[var(--accent-background)] transition-colors cursor-pointer"
                    >
                      <input
                        type="checkbox"
                        className="mt-1 accent-[var(--accent)] cursor-pointer"
                        checked={!!selectedFileKeys[file.key]}
                        onChange={() => handleFileCheckboxChange(file.key)}
                      />
                      <div className="flex-1 flex flex-col">
                        <div className="flex items-center justify-between">
                          <span className="text-sm font-bold text-[var(--foreground)]">{file.label}</span>
                          <span className="text-[10px] font-mono bg-[var(--accent-background)] px-1.5 py-0.5 border border-[var(--border-strong)] text-[var(--foreground-muted)]">
                            {formatBytes(file.size_bytes)}
                          </span>
                        </div>
                        <span className="text-[11px] text-[var(--foreground-muted)] leading-relaxed mt-1">
                          {file.description}
                        </span>
                      </div>
                    </label>
                  ))}
                  {showJournalPanel && (
                    <div className="overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-strong)]">
                      <button
                        type="button"
                        onClick={() => setJournalsExpanded((current) => !current)}
                        className="flex w-full items-center justify-between gap-3 bg-[var(--accent-background)] px-3 py-3 text-left text-[var(--foreground)] transition-colors hover:bg-[var(--accent-hover)]"
                        aria-expanded={journalsExpanded}
                      >
                        <span className="min-w-0">
                          <span className="block text-sm font-bold">{t("import.journals_title")}</span>
                          <span className="block truncate text-[11px] font-medium text-[var(--foreground-muted)]">
                            {logImportRange === "none"
                              ? t("import.journals_clean")
                              : t("import.journals_selected", {
                                  selected: selectedJournalCount,
                                  total: journalFiles.length,
                                })}
                          </span>
                        </span>
                        <ChevronIcon expanded={journalsExpanded} />
                      </button>
                      {journalsExpanded && (
                        <div className="flex flex-col gap-3 border-t border-[var(--border-strong)] p-3">
                          <label className="flex flex-col gap-1.5">
                            <span className="text-xs font-semibold text-[var(--foreground-muted)]">
                              {t("import.journals_range")}
                            </span>
                            <select
                              className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)] cursor-pointer"
                              value={logImportRange}
                              onChange={(event) =>
                                void handleLogImportRangeChange(event.target.value as LogImportRange)
                              }
                              disabled={isScanning || isImporting}
                            >
                              {LOG_IMPORT_RANGE_OPTIONS.map((option) => (
                                <option key={option.value} value={option.value}>
                                  {t(option.labelKey)}
                                </option>
                              ))}
                            </select>
                          </label>
                          {journalFiles.length === 0 ? (
                            <div className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-4 text-center text-xs text-[var(--foreground-muted)]">
                              {t("import.journals_no_past")}
                            </div>
                          ) : (
                            <div className="flex flex-col gap-2">
                              {journalFiles.map((file, index) => (
                                <label
                                  key={file.key}
                                  className="flex items-start gap-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] p-3 hover:bg-[var(--accent-background)] transition-colors cursor-pointer"
                                >
                                  <input
                                    type="checkbox"
                                    className="mt-1 accent-[var(--accent)] cursor-pointer"
                                    checked={!!selectedFileKeys[file.key]}
                                    onChange={() => handleFileCheckboxChange(file.key)}
                                  />
                                  <div className="min-w-0 flex-1 flex flex-col">
                                    <div className="flex items-center justify-between gap-3">
                                      <span className="truncate text-sm font-bold text-[var(--foreground)]">
                                        {journalDisplayName(file, index, t)}
                                      </span>
                                      <span className="shrink-0 text-[10px] font-mono bg-[var(--accent-background)] px-1.5 py-0.5 border border-[var(--border-strong)] text-[var(--foreground-muted)]">
                                        {formatBytes(file.size_bytes)}
                                      </span>
                                    </div>
                                    <span className="text-[11px] text-[var(--foreground-muted)] leading-relaxed mt-1">
                                      {file.description}
                                    </span>
                                  </div>
                                </label>
                              ))}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* OOMU Specific Configurations */}
            {refreshTarget ? (
              <div className="h-fit rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-6">
                <h3 className="text-sm font-semibold text-[var(--foreground)]">
                  {refreshTarget.name}
                </h3>
                <p className="mt-2 text-xs leading-relaxed text-[var(--foreground-muted)]">
                  {t("sprint_299.import_refresh.setup_kept")}
                </p>
              </div>
            ) : (
            <div className="rounded-[var(--radius-md)] border border-[var(--border-strong)] p-6 flex flex-col gap-4 bg-[var(--background)] h-fit">
              <h3 className="text-sm font-semibold text-[var(--foreground)]">
                {t("import.step_setup")}
              </h3>
              <p className="text-xs text-[var(--foreground-muted)] leading-relaxed mb-2">
                {t("import.step_setup_desc")}
              </p>

              <label className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("import.name_label")}</span>
                <input
                  type="text"
                  required
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  placeholder={t("import.name_placeholder")}
                  value={agentName}
                  onChange={(e) => setAgentName(e.target.value)}
                />
              </label>

              <label className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("import.desc_label")}</span>
                <textarea
                  required
                  className="h-20 resize-none border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium leading-relaxed text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)]"
                  placeholder={t("import.desc_placeholder")}
                  value={agentDescription}
                  onChange={(e) => setAgentDescription(e.target.value)}
                />
              </label>

              <label className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("import.style_label")}</span>
                <select
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)] cursor-pointer"
                  value={selectedTemplate}
                  onChange={(e) => setSelectedTemplate(e.target.value)}
                >
                  {templateOptions.map((opt) => (
                    <option key={opt.id} value={opt.id}>
                      {opt.name}
                    </option>
                  ))}
                </select>
              </label>

              <div className="grid grid-cols-2 gap-4">
                <label className="flex flex-col gap-1.5">
                  <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("import.provider_label")}</span>
                  <select
                    className="border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)] cursor-pointer"
                    value={selectedProviderId}
                    onChange={(e) => handleProviderChange(e.target.value)}
                  >
                    {providerOptions.map((opt) => (
                      <option key={opt.id} value={opt.id}>
                        {opt.name}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="flex flex-col gap-1.5">
                  <span className="text-xs font-medium text-[var(--foreground-muted)]">{t("import.model_label")}</span>
                  <select
                    className="border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm text-[var(--foreground)] outline-none focus:bg-[var(--accent-background)] cursor-pointer"
                    value={selectedModelId}
                    onChange={(e) => setSelectedModelId(e.target.value)}
                  >
                    {activeProvider && activeProvider.customModelIds.length > 0 ? (
                      activeProvider.customModelIds.map((m: string) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))
                    ) : (
                      <option value={DEFAULT_LOCAL_MODEL_ID}>{DEFAULT_LOCAL_MODEL_ID}</option>
                    )}
                  </select>
                </label>
              </div>
            </div>
            )}
          </div>

          {importError && (
            <div className="text-[var(--destructive)] text-xs font-semibold border-l-2 border-[var(--destructive)] pl-3">
              {t("import.import_failed", { error: importError })}
            </div>
          )}

          <div className="flex justify-end gap-3 border-t border-[var(--border-strong)] pt-4 mt-2">
            <button
              type="button"
              disabled={isImporting}
              onClick={onCancel}
              className="border border-[var(--border-strong)] bg-transparent px-6 py-2 text-sm font-medium text-[var(--foreground)] hover:bg-[var(--accent-background)] transition-colors disabled:opacity-50"
            >
              {t("common.cancel")}
            </button>
            <button
              type="submit"
              disabled={isImporting || !agentName.trim()}
              className="bg-[var(--inverse-background)] text-[var(--inverse-foreground)] px-8 py-2 text-sm font-semibold hover:bg-[var(--accent-hover)] transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isImporting
                ? t(refreshTarget ? "sprint_299.import_refresh.working_button" : "import.importing_button")
                : t(refreshTarget ? "sprint_299.import_refresh.complete_button" : "import.complete_button")}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
