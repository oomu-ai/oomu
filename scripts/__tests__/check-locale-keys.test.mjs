import { describe, expect, it, vi } from "vitest";
import {
  GERMAN_WORKFLOW_STUB_PATTERN,
  blockingIssueCount,
  evaluateLocale,
  isSuspiciousEnglishFallback,
  runLocaleCheck,
  workflowStubPaths,
} from "../check-locale-keys.mjs";

// Regression lock for the localization guard. The audit history records this
// guard being faked twice: first German stubs shipped in non-German locales,
// then the "fix" used `\\b` (a literal backslash) instead of a `\b` word
// boundary, so it matched nothing and silently passed. These tests fail if
// either regression returns.
describe("i18n Workflows localization guard", () => {
  it("uses a real word boundary so it actually catches German stubs", () => {
    // Would be false if the pattern reverted to the broken `\\b` form.
    expect(GERMAN_WORKFLOW_STUB_PATTERN.test("Wird erstellt…")).toBe(true);
    expect(GERMAN_WORKFLOW_STUB_PATTERN.test("Dieser Workflow braucht …")).toBe(true);
    expect(GERMAN_WORKFLOW_STUB_PATTERN.test("Gespeichert")).toBe(true);
    // and does not flag genuine English copy as German
    expect(GERMAN_WORKFLOW_STUB_PATTERN.test("Review the steps, then save.")).toBe(false);
  });

  it("flags a German stub copied into a non-German locale, but exempts de-DE", () => {
    const reference = { workflows: { composer: { running: "Running" } } };
    const stubbed = { workflows: { composer: { running: "Wird erstellt…" } } };
    expect(workflowStubPaths(stubbed, reference, "es-ES.json")).toEqual([
      "workflows.composer.running",
    ]);
    // de-DE legitimately contains German and must never be flagged.
    expect(workflowStubPaths(stubbed, reference, "de-DE.json")).toEqual([]);
  });

  it("flags an untranslated English fallback but allows real translations", () => {
    expect(
      isSuspiciousEnglishFallback("Reviewing the steps, then save.", "Reviewing the steps, then save."),
    ).toBe(true);
    expect(
      isSuspiciousEnglishFallback("Revisa los pasos y luego guarda.", "Reviewing the steps, then save."),
    ).toBe(false);
    // Short strings are not treated as fallbacks (proper nouns, single words).
    expect(isSuspiciousEnglishFallback("Mail", "Mail")).toBe(false);
  });

  it("treats missing keys as non-blocking (English-first policy) but stubs as blocking", () => {
    const reference = {
      workflows: { composer: { running: "Running the workflow steps now." } },
      chat: { send: "Send" },
    };
    // A locale missing a key falls back to English at runtime — warn, don't fail.
    const sparse = { workflows: { composer: {} }, chat: {} };
    const sparseEvaluation = evaluateLocale(sparse, reference, "ja-JP.json");
    expect(sparseEvaluation.missing).toEqual(["workflows.composer.running", "chat.send"]);
    expect(blockingIssueCount(sparseEvaluation)).toBe(0);
    // A German stub pasted into the wrong locale is a bad entry — still fails.
    const stubbed = {
      workflows: { composer: { running: "Wird erstellt…" } },
      chat: { send: "Send" },
    };
    expect(blockingIssueCount(evaluateLocale(stubbed, reference, "es-ES.json"))).toBe(1);
  });

  it("passes against the real locale files with zero blocking issues", () => {
    const spies = ["log", "error", "warn"].map((method) =>
      vi.spyOn(console, method).mockImplementation(() => {}),
    );
    try {
      expect(runLocaleCheck()).toBe(0);
    } finally {
      spies.forEach((spy) => spy.mockRestore());
    }
  });
});
