"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@/lib/invoke";
import enUS from "@/locales/en-US.json";

type TranslationValue = string | TranslationRecord;
type TranslationRecord = { [key: string]: TranslationValue };

type LocaleOption = {
  id: string;
  label: string;
  fileName: string;
  isDefault: boolean;
  verified: boolean;
};

type LocaleStatePayload = {
  activeLocale: string;
  availableLocales: LocaleOption[];
  translations: unknown;
};

type I18nContextType = {
  language: string;
  availableLocales: LocaleOption[];
  isLoadingLocales: boolean;
  isChangingLanguage: boolean;
  localeError: string | null;
  setLanguage: (lang: string) => Promise<void>;
  t: (key: string, variables?: Record<string, string | number>) => string;
};

const DEFAULT_LOCALE_ID = "en-US";
const DEFAULT_TRANSLATIONS = enUS as TranslationRecord;
const DEFAULT_LOCALE_OPTION: LocaleOption = {
  id: DEFAULT_LOCALE_ID,
  label: "English (US)",
  fileName: "en-US.json",
  isDefault: true,
  verified: true,
};

const I18nContext = createContext<I18nContextType | null>(null);

function isTranslationRecord(value: unknown): value is TranslationRecord {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function mergeTranslationRecords(
  defaults: TranslationRecord,
  active: TranslationRecord,
): TranslationRecord {
  const merged: TranslationRecord = { ...defaults };

  Object.entries(active).forEach(([key, value]) => {
    const defaultValue = defaults[key];
    merged[key] =
      isTranslationRecord(defaultValue) && isTranslationRecord(value)
        ? mergeTranslationRecords(defaultValue, value)
        : value;
  });

  return merged;
}

function verifiedLocaleOptions(locales: LocaleOption[]) {
  const verified = locales.filter((locale) => locale.verified);
  return verified.some((locale) => locale.id === DEFAULT_LOCALE_ID)
    ? verified
    : [DEFAULT_LOCALE_OPTION];
}

function resolveTranslation(root: TranslationRecord | undefined, key: string) {
  if (!root) return undefined;

  let node: TranslationValue | undefined = root;
  for (const part of key.split(".")) {
    if (!isTranslationRecord(node)) {
      return undefined;
    }
    node = node[part];
  }

  return typeof node === "string" ? node : undefined;
}

function interpolate(value: string, variables?: Record<string, string | number>) {
  if (!variables) return value;

  let result = value;
  Object.entries(variables).forEach(([key, replacement]) => {
    result = result.split(`{${key}}`).join(String(replacement));
  });
  return result;
}

function localeErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function applyDocumentLanguage(language: string) {
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute("lang", language);
  }
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [language, setLanguageState] = useState(DEFAULT_LOCALE_ID);
  const [availableLocales, setAvailableLocales] = useState<LocaleOption[]>([
    DEFAULT_LOCALE_OPTION,
  ]);
  const [activeTranslations, setActiveTranslations] =
    useState<TranslationRecord>(DEFAULT_TRANSLATIONS);
  const [isLoadingLocales, setIsLoadingLocales] = useState(true);
  const [isChangingLanguage, setIsChangingLanguage] = useState(false);
  const [localeError, setLocaleError] = useState<string | null>(null);

  const applyLocaleState = useCallback((payload: LocaleStatePayload) => {
    const verifiedLocales = verifiedLocaleOptions(payload.availableLocales);
    const activeLocale = verifiedLocales.some(
      (locale) => locale.id === payload.activeLocale,
    )
      ? payload.activeLocale
      : DEFAULT_LOCALE_ID;
    const translations =
      activeLocale !== DEFAULT_LOCALE_ID && isTranslationRecord(payload.translations)
        ? mergeTranslationRecords(DEFAULT_TRANSLATIONS, payload.translations)
        : DEFAULT_TRANSLATIONS;
    applyDocumentLanguage(activeLocale);
    setAvailableLocales(verifiedLocales);
    setLanguageState(activeLocale);
    setActiveTranslations(translations);
    setLocaleError(null);
  }, []);

  useEffect(() => {
    let active = true;

    invoke<LocaleStatePayload>("get_locale_state")
      .then((payload) => {
        if (!active) return;
        applyLocaleState(payload);
      })
      .catch((error: unknown) => {
        if (!active) return;
        setLocaleError(localeErrorMessage(error));
      })
      .finally(() => {
        if (!active) return;
        setIsLoadingLocales(false);
      });

    return () => {
      active = false;
    };
  }, [applyLocaleState]);

  useEffect(() => {
    if (typeof document === "undefined") return;

    const documentElement = document.documentElement;
    const previousLanguage = documentElement.getAttribute("lang");

    return () => {
      if (previousLanguage === null) {
        documentElement.removeAttribute("lang");
      } else {
        documentElement.setAttribute("lang", previousLanguage);
      }
    };
  }, []);

  useEffect(() => {
    applyDocumentLanguage(language);
  }, [language]);

  const setLanguage = useCallback(
    async (lang: string) => {
      const requested = lang.trim();
      if (!availableLocales.some((locale) => locale.id === requested)) {
        setLocaleError(`Locale '${requested}' is not available.`);
        return;
      }

      setIsChangingLanguage(true);
      try {
        const payload = await invoke<LocaleStatePayload>("set_active_locale", {
          localeId: requested,
          locale_id: requested,
        });
        applyLocaleState(payload);
      } catch (error: unknown) {
        setLocaleError(localeErrorMessage(error));
      } finally {
        setIsChangingLanguage(false);
      }
    },
    [applyLocaleState, availableLocales],
  );

  const t = useCallback(
    (key: string, variables?: Record<string, string | number>): string => {
      const value =
        resolveTranslation(activeTranslations, key) ??
        resolveTranslation(DEFAULT_TRANSLATIONS, key);

      return interpolate(value ?? key, variables);
    },
    [activeTranslations],
  );

  return (
    <I18nContext.Provider
      value={{
        language,
        availableLocales,
        isLoadingLocales,
        isChangingLanguage,
        localeError,
        setLanguage,
        t,
      }}
    >
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used within an I18nProvider");
  }
  return context;
}
