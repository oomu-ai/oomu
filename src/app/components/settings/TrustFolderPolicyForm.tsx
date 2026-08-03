import type { ChangeEvent } from "react";

export type TrustPermissionLevel = "session_gated" | "global_trust";
export type TrustToolCategory = "shell_commands" | "external_writes";

type TrustTranslate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export const TRUST_TOOL_CATEGORIES: {
  id: TrustToolCategory;
  labelKey: string;
}[] = [
  { id: "shell_commands", labelKey: "settings.privacy.trust.tool_shell" },
  { id: "external_writes", labelKey: "settings.privacy.trust.tool_writes" },
];

const TRUST_TIER_OPTIONS: {
  id: TrustPermissionLevel;
  labelKey: string;
}[] = [
  { id: "global_trust", labelKey: "settings.privacy.trust.tier_global" },
  { id: "session_gated", labelKey: "settings.privacy.trust.tier_session" },
];

export const DEFAULT_TRUST_PATH = "~/Projects/OOMU";

export const secondaryButtonClass =
  "inline-flex h-9 shrink-0 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50";

const primaryButtonClass =
  "inline-flex h-10 shrink-0 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50";

type TrustFolderPolicyFormProps = {
  browseState: "idle" | "working" | "success" | "error";
  categorySelection: Record<TrustToolCategory, boolean>;
  directoryPath: string;
  isBrowsing: boolean;
  isSaving: boolean;
  onBrowse: () => void;
  onCategoryToggle: (category: TrustToolCategory) => void;
  onDirectoryPathChange: (path: string) => void;
  onSave: () => void;
  onTrustTierChange: (tier: TrustPermissionLevel) => void;
  saveState: "idle" | "working" | "success" | "error";
  t: TrustTranslate;
  trustTier: TrustPermissionLevel;
};

export function TrustFolderPolicyForm({
  browseState,
  categorySelection,
  directoryPath,
  isBrowsing,
  isSaving,
  onBrowse,
  onCategoryToggle,
  onDirectoryPathChange,
  onSave,
  onTrustTierChange,
  saveState,
  t,
  trustTier,
}: TrustFolderPolicyFormProps) {
  const handlePathChange = (event: ChangeEvent<HTMLInputElement>) => {
    onDirectoryPathChange(event.target.value);
  };

  return (
    <div className="mt-5 grid gap-3 xl:grid-cols-[minmax(0,1fr)_11rem_18rem_8rem]">
      <div>
        <label
          className="mb-1 block text-xs font-semibold text-[var(--foreground-muted)]"
          htmlFor="trust-directory-path"
        >
          {t("settings.privacy.trust.folder_path")}
        </label>
        <div className="flex min-w-0 gap-2">
          <input
            className="h-10 min-w-0 flex-1 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 font-mono text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-muted)] focus:border-[var(--foreground-muted)]"
            id="trust-directory-path"
            onChange={handlePathChange}
            placeholder="~/Projects/OOMU"
            spellCheck={false}
            value={directoryPath}
          />
          <button
            aria-busy={isBrowsing}
            className={secondaryButtonClass}
            data-action-state={browseState}
            disabled={isBrowsing || isSaving}
            onClick={onBrowse}
            type="button"
          >
            {isBrowsing ? t("common.browsing") : t("common.browse")}
          </button>
        </div>
      </div>
      <div>
        <label
          className="mb-1 block text-xs font-semibold text-[var(--foreground-muted)]"
          htmlFor="trust-tier"
        >
          {t("settings.privacy.trust.trust_tier")}
        </label>
        <select
          className="h-10 w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-sm font-medium text-[var(--foreground)] outline-none focus:border-[var(--foreground-muted)]"
          id="trust-tier"
          onChange={(event) =>
            onTrustTierChange(event.target.value as TrustPermissionLevel)
          }
          value={trustTier}
        >
          {TRUST_TIER_OPTIONS.map((option) => (
            <option key={option.id} value={option.id}>
              {t(option.labelKey)}
            </option>
          ))}
        </select>
      </div>
      <fieldset className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2">
        <legend className="px-1 text-xs font-semibold text-[var(--foreground-muted)]">
          {t("settings.privacy.trust.allowed_tools")}
        </legend>
        <div className="flex flex-wrap gap-3">
          {TRUST_TOOL_CATEGORIES.map((category) => (
            <label
              className="flex items-center gap-2 text-xs font-medium text-[var(--foreground)]"
              key={category.id}
            >
              <input
                checked={categorySelection[category.id]}
                className="h-4 w-4"
                onChange={() => onCategoryToggle(category.id)}
                type="checkbox"
              />
              {t(category.labelKey)}
            </label>
          ))}
        </div>
      </fieldset>
      <div className="flex items-end">
        <button
          aria-busy={isSaving}
          className={primaryButtonClass}
          data-action-state={saveState}
          disabled={isSaving}
          onClick={onSave}
          type="button"
        >
          {isSaving ? t("common.saving") : t("common.save")}
        </button>
      </div>
    </div>
  );
}
