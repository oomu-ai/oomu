import { useI18n } from "@/context/I18nContext";
import type { ConfiguredModelOption } from "@/lib/modelRegistry";

type Props = {
  models: ConfiguredModelOption[];
  value: string;
  verifiedStartupModelId: string | null;
  onChange: (modelId: string) => void;
};

export function NewAgentModelSelect({
  models,
  value,
  verifiedStartupModelId,
  onChange,
}: Props) {
  const { t } = useI18n();
  const verifiedModelIsListed = models.some(
    (model) => model.modelId === verifiedStartupModelId,
  );

  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-[var(--foreground-muted)]">
        {t("agents.new_agent_dialog.model")}
      </span>
      <select
        className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)] disabled:text-[var(--foreground-muted)]"
        disabled={models.length === 0 && !verifiedStartupModelId}
        onChange={(event) => onChange(event.target.value)}
        value={value}
      >
        {!value && (
          <option disabled value="">
            {t("agents.profile.configure_model_first")}
          </option>
        )}
        {verifiedStartupModelId && !verifiedModelIsListed && (
          <option value={verifiedStartupModelId}>
            {verifiedStartupModelId} ({t("agents.new_agent_dialog.default_badge")})
          </option>
        )}
        {models.map((model) => (
          <option key={`${model.providerId}-${model.modelId}`} value={model.modelId}>
            {model.modelId}
          </option>
        ))}
      </select>
    </label>
  );
}
