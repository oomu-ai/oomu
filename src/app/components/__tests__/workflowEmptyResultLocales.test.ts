import { describe, expect, it } from "vitest";
import deDE from "@/locales/de-DE.json";
import enUS from "@/locales/en-US.json";
import esES from "@/locales/es-ES.json";
import frFR from "@/locales/fr-FR.json";
import idID from "@/locales/id-ID.json";
import jaJP from "@/locales/ja-JP.json";
import ptBR from "@/locales/pt-BR.json";
import ruRU from "@/locales/ru-RU.json";
import ukUA from "@/locales/uk-UA.json";
import viVN from "@/locales/vi-VN.json";
import zhCN from "@/locales/zh-CN.json";
import zhTW from "@/locales/zh-TW.json";

const EMPTY_RESULT_KEYS = [
  "workflows.composer.notice_saved",
  "workflows.library.completed_empty_status",
  "workflows.library.completed_empty_toast",
  "workflows.run.empty_result",
  "workflows.run.status.completed_empty",
  "workflows.templates.empty_result.label",
  "workflows.templates.empty_result.check_files",
  "workflows.templates.empty_result.check_events",
  "workflows.templates.empty_result.check_mail",
  "workflows.templates.empty_result.check_reminders",
] as const;

const LOCALES = [
  ["de-DE", deDE],
  ["en-US", enUS],
  ["es-ES", esES],
  ["fr-FR", frFR],
  ["id-ID", idID],
  ["ja-JP", jaJP],
  ["pt-BR", ptBR],
  ["ru-RU", ruRU],
  ["uk-UA", ukUA],
  ["vi-VN", viVN],
  ["zh-CN", zhCN],
  ["zh-TW", zhTW],
] as const;

function translationAt(messages: unknown, key: string): unknown {
  return key.split(".").reduce<unknown>((value, part) => {
    if (!value || typeof value !== "object") return undefined;
    return (value as Record<string, unknown>)[part];
  }, messages);
}

describe("workflow empty-result locale contract", () => {
  it("defines every empty-result message in every supported locale", () => {
    expect(LOCALES).toHaveLength(12);

    for (const [locale, messages] of LOCALES) {
      for (const key of EMPTY_RESULT_KEYS) {
        const value = translationAt(messages, key);
        expect(typeof value, `${locale}: ${key}`).toBe("string");
        expect((value as string).trim(), `${locale}: ${key}`).not.toBe("");
      }
    }
  });

  it("preserves the workflow name placeholder in every locale", () => {
    for (const [locale, messages] of LOCALES) {
      const value = translationAt(
        messages,
        "workflows.library.completed_empty_status",
      );
      expect(value, `${locale}: completed_empty_status`).toContain("{name}");
    }
  });

  it("does not ship English fallback copy in non-English locales", () => {
    for (const [locale, messages] of LOCALES) {
      if (locale === "en-US") continue;

      for (const key of EMPTY_RESULT_KEYS) {
        expect(translationAt(messages, key), `${locale}: ${key}`).not.toBe(
          translationAt(enUS, key),
        );
      }
    }
  });
});
