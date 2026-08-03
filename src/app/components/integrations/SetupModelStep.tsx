"use client";

import { useI18n } from "@/context/I18nContext";
import type { ConfiguredProvider } from "@/lib/modelRegistry";
import { RecommendedLocalModelSetup } from "./RecommendedLocalModelSetup";
import type { RecommendedModelProviderEvidence } from "./useRecommendedLocalModelInstall";

type LocalModel = {
  id: string;
  compatibility: string;
};

type SetupModelStepProps = {
  busy: boolean;
  localModels: LocalModel[];
  modelRoute: string;
  onChooseExisting: () => void | Promise<void>;
  onDefer: () => void | Promise<void>;
  onRouteChange: (route: string) => void;
  onUseExisting: () => void | Promise<void>;
  onVerified: (evidence: RecommendedModelProviderEvidence) => void | Promise<void>;
  providers: ConfiguredProvider[];
};

export function SetupModelStep({
  busy,
  localModels,
  modelRoute,
  onChooseExisting,
  onDefer,
  onRouteChange,
  onUseExisting,
  onVerified,
  providers,
}: SetupModelStepProps) {
  const { t } = useI18n();
  return (
    <div className="grid gap-3">
      <label className="rounded border p-4">
        <input
          checked={modelRoute === "local"}
          name="modelRoute"
          onChange={() => onRouteChange("local")}
          type="radio"
        />
        <span className="ml-2 font-semibold">{t("setup.local_model")}</span>
        <p className="ml-6 mt-1 text-sm text-[var(--foreground-muted)]">
          {t("setup.local_model_help")}
        </p>
      </label>
      {modelRoute === "local" ? (
        <RecommendedLocalModelSetup
          disabled={busy}
          hasExistingReadyModel={localModels.some(
            (model) => model.compatibility === "ready",
          )}
          onChooseExisting={onChooseExisting}
          onDefer={onDefer}
          onUseExisting={onUseExisting}
          onVerified={onVerified}
        />
      ) : null}
      {providers.map((provider) => (
        <label className="rounded border p-4" key={provider.id}>
          <input
            checked={modelRoute === provider.id}
            name="modelRoute"
            onChange={() => onRouteChange(provider.id)}
            type="radio"
          />
          <span className="ml-2 font-semibold">{provider.providerName}</span>
          <p className="ml-6 mt-1 text-sm text-[var(--foreground-muted)]">
            {t("setup.cloud_model_help")}
          </p>
        </label>
      ))}
    </div>
  );
}
