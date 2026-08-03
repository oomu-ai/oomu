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

const locales = [
  ["en-US", enUS],
  ["de-DE", deDE],
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

const nameKeys = [
  "name_label",
  "name_help",
  "name_placeholder",
  "name_required",
] as const;

describe("workflow composer naming translations", () => {
  it.each(locales)("ships complete native naming copy for %s", (locale, messages) => {
    for (const key of nameKeys) {
      const value = messages.workflows.composer[key];
      expect(value, `${locale}.${key}`).toEqual(expect.any(String));
      expect(value.trim(), `${locale}.${key}`).not.toBe("");
      if (locale !== "en-US") {
        expect(value, `${locale}.${key}`).not.toBe(enUS.workflows.composer[key]);
      }
    }
  });
});
