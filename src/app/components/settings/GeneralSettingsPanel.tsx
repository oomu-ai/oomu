"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTheme } from "@/components/ThemeProvider";
import { invoke } from "@/lib/invoke";
import { useI18n } from "@/context/I18nContext";
import { PresentationCheckerSetup } from "../artifacts/presentations/PresentationCheckerSetup";

type ModelDirectoryStatus =
  | { key: string; message?: never }
  | { key?: never; message: string };

type LocalModelOption = {
  id: string;
  name: string;
  compatibility: "ready" | "unsupported" | "invalid" | "asset_missing";
};

type DefaultPrewarmedModelSetting = {
  modelId: string;
  isDefault: boolean;
};

type AppearanceTheme = "light" | "dark" | "system";
type ResolvedAppearanceTheme = Exclude<AppearanceTheme, "system">;

const appearanceThemeLabelKeys: Record<AppearanceTheme, string> = {
  light: "settings.general.appearance.theme_options.light",
  dark: "settings.general.appearance.theme_options.dark",
  system: "settings.general.appearance.theme_options.system",
};

function errorMessage(error: unknown, fallback: string) {
  return error && typeof error === "object" && "message" in error
    ? String(error.message)
    : fallback;
}

export function GeneralSettingsPanel() {
  const { theme, setTheme, resolvedTheme } = useTheme();
  const {
    language,
    availableLocales,
    isLoadingLocales,
    isChangingLanguage,
    localeError,
    setLanguage,
    t,
  } = useI18n();
  const [localModelDirectory, setLocalModelDirectory] = useState("");
  const [isDefaultModelDirectory, setIsDefaultModelDirectory] = useState(true);
  const [modelDirectoryStatus, setModelDirectoryStatus] =
    useState<ModelDirectoryStatus>({
      key: "settings.general.model_directory.loading",
    });
  const [isBrowsingModelDirectory, setIsBrowsingModelDirectory] =
    useState(false);
  const [localModels, setLocalModels] = useState<LocalModelOption[]>([]);
  const [isLoadingLocalModels, setIsLoadingLocalModels] = useState(false);
  const [prewarmedModelId, setPrewarmedModelId] = useState("");
  const [isDefaultPrewarmedModel, setIsDefaultPrewarmedModel] = useState(true);
  const [isSavingPrewarmedModel, setIsSavingPrewarmedModel] = useState(false);
  const [prewarmedModelStatus, setPrewarmedModelStatus] =
    useState<ModelDirectoryStatus>({
      key: "settings.general.default_prewarmed_model.loading",
    });
  const appearanceThemeLabel = (appearanceTheme: AppearanceTheme) =>
    t(appearanceThemeLabelKeys[appearanceTheme]);
  const themeOptions = [
    { id: "light", label: appearanceThemeLabel("light") },
    { id: "dark", label: appearanceThemeLabel("dark") },
    { id: "system", label: appearanceThemeLabel("system") },
  ] as const;
  const resolvedAppearanceTheme = resolvedTheme as ResolvedAppearanceTheme;
  const readyLocalModels = useMemo(
    () => localModels.filter((model) => model.compatibility === "ready"),
    [localModels],
  );
  const selectedPrewarmedModelIsAvailable = readyLocalModels.some(
    (model) => model.id === prewarmedModelId,
  );

  const refreshLocalModels = useCallback(
    async (isActive: () => boolean = () => true) => {
      setIsLoadingLocalModels(true);
      try {
        const models = await invoke<LocalModelOption[]>("list_local_models");
        if (!isActive()) return;
        setLocalModels(models);
      } catch (error) {
        if (!isActive()) return;
        setLocalModels([]);
        setPrewarmedModelStatus({
          message: errorMessage(
            error,
            t("settings.general.default_prewarmed_model.models_load_error"),
          ),
        });
      } finally {
        if (isActive()) {
          setIsLoadingLocalModels(false);
        }
      }
    },
    [t],
  );

  useEffect(() => {
    let active = true;

    invoke<{ path: string; isDefault: boolean }>("get_local_model_directory")
      .then((setting) => {
        if (!active) return;
        setLocalModelDirectory(setting.path);
        setIsDefaultModelDirectory(setting.isDefault);
        setModelDirectoryStatus(
          setting.isDefault
            ? { key: "settings.general.model_directory.default_status" }
            : { key: "settings.general.model_directory.saved_status" },
        );
      })
      .catch((error) => {
        if (!active) return;
        setModelDirectoryStatus({
          message: errorMessage(
            error,
            t("settings.general.model_directory.load_error"),
          ),
        });
      });

    return () => {
      active = false;
    };
  }, [t]);

  useEffect(() => {
    let active = true;

    invoke<DefaultPrewarmedModelSetting>("get_default_prewarmed_model")
      .then((setting) => {
        if (!active) return;
        setPrewarmedModelId(setting.modelId);
        setIsDefaultPrewarmedModel(setting.isDefault);
        setPrewarmedModelStatus(
          setting.isDefault
            ? { key: "settings.general.default_prewarmed_model.default_status" }
            : { key: "settings.general.default_prewarmed_model.saved_status" },
        );
      })
      .catch((error) => {
        if (!active) return;
        setPrewarmedModelStatus({
          message: errorMessage(
            error,
            t("settings.general.default_prewarmed_model.load_error"),
          ),
        });
      });

    return () => {
      active = false;
    };
  }, [t]);

  useEffect(() => {
    let active = true;
    const timeout = window.setTimeout(() => {
      void refreshLocalModels(() => active);
    }, 0);

    return () => {
      active = false;
      window.clearTimeout(timeout);
    };
  }, [refreshLocalModels]);

  const browseForModelDirectory = async () => {
    setIsBrowsingModelDirectory(true);
    setModelDirectoryStatus({
      key: "settings.general.model_directory.waiting_status",
    });

    try {
      const setting = await invoke<{ path: string; isDefault: boolean } | null>(
        "choose_local_model_directory",
      );
      if (setting) {
        setLocalModelDirectory(setting.path);
        setIsDefaultModelDirectory(setting.isDefault);
        setModelDirectoryStatus({
          key: "settings.general.model_directory.saved_status",
        });
        await refreshLocalModels();
      } else {
        setModelDirectoryStatus(
          isDefaultModelDirectory
            ? { key: "settings.general.model_directory.default_status" }
            : { key: "settings.general.model_directory.unchanged_status" },
        );
      }
    } catch (error) {
      setModelDirectoryStatus({
        message: errorMessage(
          error,
          t("settings.general.model_directory.choose_error"),
        ),
      });
    } finally {
      setIsBrowsingModelDirectory(false);
    }
  };

  const saveDefaultPrewarmedModel = async (modelId: string) => {
    if (!modelId || modelId === prewarmedModelId) return;

    const previousModelId = prewarmedModelId;
    const previousIsDefault = isDefaultPrewarmedModel;
    setPrewarmedModelId(modelId);
    setIsDefaultPrewarmedModel(false);
    setIsSavingPrewarmedModel(true);
    setPrewarmedModelStatus({
      key: "settings.general.default_prewarmed_model.saving_status",
    });

    try {
      const setting = await invoke<DefaultPrewarmedModelSetting>(
        "set_default_prewarmed_model",
        { modelId, model_id: modelId },
      );
      setPrewarmedModelId(setting.modelId);
      setIsDefaultPrewarmedModel(setting.isDefault);
      setPrewarmedModelStatus({
        key: "settings.general.default_prewarmed_model.saved_status",
      });
    } catch (error) {
      setPrewarmedModelId(previousModelId);
      setIsDefaultPrewarmedModel(previousIsDefault);
      setPrewarmedModelStatus({
        message: errorMessage(
          error,
          t("settings.general.default_prewarmed_model.save_error"),
        ),
      });
    } finally {
      setIsSavingPrewarmedModel(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <h2 className="text-sm font-semibold text-[var(--foreground)]">
              {t("settings.general.appearance.title")}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
              {t("settings.general.appearance.description", {
                theme: appearanceThemeLabel(resolvedAppearanceTheme),
              })}
            </p>
          </div>
          <div className="grid w-full grid-cols-3 overflow-hidden rounded-[var(--radius-sm)] border border-[var(--border-strong)] lg:w-[24rem]">
            {themeOptions.map((option) => {
              const selected = theme === option.id;

              return (
                <button
                  aria-pressed={selected}
                  className={`h-11 border-r border-[var(--border-strong)] px-4 text-sm font-medium transition-colors last:border-r-0 ${
                    selected
                      ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
                      : "bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                  }`}
                  key={option.id}
                  onClick={() => setTheme(option.id)}
                  type="button"
                >
                  {option.label}
                </button>
              );
            })}
          </div>
        </div>
      </section>

      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <h2 className="text-sm font-semibold text-[var(--foreground)]">
              {t("settings.general.language.title")}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
              {t("settings.general.language.description")}
            </p>
          </div>
          <div className="relative w-full lg:w-[24rem]">
            <select
              aria-label={t("settings.general.language.select_label")}
              className="h-11 w-full cursor-pointer rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 text-sm font-medium text-[var(--foreground)] outline-none transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-60"
              disabled={isLoadingLocales || isChangingLanguage}
              value={language}
              onChange={(e) => void setLanguage(e.target.value)}
            >
              {availableLocales.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
            <p
              aria-live="polite"
              className={`mt-2 text-xs font-medium ${
                localeError
                  ? "text-[var(--destructive)]"
                  : "text-[var(--foreground-muted)]"
              }`}
            >
              {localeError ??
                (isLoadingLocales
                  ? t("settings.general.language.loading")
                  : t("settings.general.language.available_count", {
                      count: availableLocales.length,
                    }))}
            </p>
          </div>
        </div>
      </section>

      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <div className="flex flex-col gap-5">
          <div>
            <h2 className="text-sm font-semibold text-[var(--foreground)]">
              {t("settings.general.model_directory.title")}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
              {t("settings.general.model_directory.description")}
            </p>
          </div>
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
            <label className="sr-only" htmlFor="local-model-directory">
              {t("settings.general.model_directory.title")}
            </label>
            <input
              className="h-11 min-w-0 flex-1 cursor-pointer border border-[var(--border-strong)] bg-[var(--background)] px-3 font-mono text-sm text-[var(--foreground)] outline-none transition-colors hover:bg-[var(--accent-background)]"
              id="local-model-directory"
              onClick={browseForModelDirectory}
              placeholder={t("settings.general.model_directory.placeholder")}
              readOnly
              title={localModelDirectory}
              value={isDefaultModelDirectory ? "" : localModelDirectory}
            />
            <button
              className="h-11 shrink-0 bg-[var(--inverse-background)] px-6 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
              disabled={isBrowsingModelDirectory}
              onClick={browseForModelDirectory}
              type="button"
            >
              {isBrowsingModelDirectory ? t("common.browsing") : t("common.browse")}
            </button>
          </div>
          <p
            aria-live="polite"
            className="text-xs font-medium text-[var(--foreground-muted)]"
          >
            {"message" in modelDirectoryStatus
              ? modelDirectoryStatus.message
              : t(modelDirectoryStatus.key)}
          </p>
        </div>
      </section>

      <section className="rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5">
        <div className="flex flex-col gap-5">
          <div>
            <h2 className="text-sm font-semibold text-[var(--foreground)]">
              {t("settings.general.default_prewarmed_model.title")}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
              {t("settings.general.default_prewarmed_model.description")}
            </p>
          </div>
          <div className="relative w-full lg:w-[24rem]">
            <label className="sr-only" htmlFor="default-prewarmed-model">
              {t("settings.general.default_prewarmed_model.select_label")}
            </label>
            <select
              aria-label={t("settings.general.default_prewarmed_model.select_label")}
              className="h-11 w-full cursor-pointer rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 text-sm font-medium text-[var(--foreground)] outline-none transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-60"
              disabled={
                isLoadingLocalModels ||
                isSavingPrewarmedModel ||
                readyLocalModels.length === 0
              }
              id="default-prewarmed-model"
              onChange={(event) => void saveDefaultPrewarmedModel(event.target.value)}
              value={prewarmedModelId}
            >
              {prewarmedModelId && !selectedPrewarmedModelIsAvailable ? (
                <option value={prewarmedModelId}>
                  {t("settings.general.default_prewarmed_model.unavailable_option", {
                    modelId: prewarmedModelId,
                  })}
                </option>
              ) : null}
              {readyLocalModels.length > 0 ? (
                readyLocalModels.map((model) => (
                  <option key={model.id} value={model.id}>
                    {model.name}
                  </option>
                ))
              ) : (
                <option value="">
                  {isLoadingLocalModels
                    ? t("settings.general.default_prewarmed_model.loading_models")
                    : t("settings.general.default_prewarmed_model.empty_models")}
                </option>
              )}
            </select>
            <p
              aria-live="polite"
              className="mt-2 text-xs font-medium text-[var(--foreground-muted)]"
            >
              {"message" in prewarmedModelStatus
                ? prewarmedModelStatus.message
                : isSavingPrewarmedModel
                  ? t("settings.general.default_prewarmed_model.saving_status")
                  : isLoadingLocalModels
                    ? t("settings.general.default_prewarmed_model.loading_models")
                    : readyLocalModels.length === 0
                      ? t("settings.general.default_prewarmed_model.empty_status")
                      : t(prewarmedModelStatus.key)}
            </p>
          </div>
        </div>
      </section>

      <PresentationCheckerSetup />
    </div>
  );
}
