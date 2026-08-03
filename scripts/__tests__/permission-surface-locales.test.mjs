import { describe, expect, it, vi } from "vitest";
import {
  permissionSurfaceIssuesForLocale,
  runPermissionSurfaceLocaleCheck,
} from "../check-permission-surface-locales.mjs";

describe("permission surface localization guard", () => {
  it("rejects missing copy, broken placeholders, and hidden English fallbacks", () => {
    const reference = {
      common: { cancel: "Cancel", close: "Close", details: "Details" },
      permissions: { title: "Use {name}?" },
    };
    const locale = {
      common: { cancel: "Cancel", close: "Cerrar", details: "Detalles" },
      permissions: { title: "¿Usar esto?" },
    };
    const issues = permissionSurfaceIssuesForLocale(locale, reference, "es-ES.json");
    expect(issues).toContain("common.cancel: untranslated English copy");
    expect(issues).toContain("permissions.title: placeholders do not match en-US");
    expect(issues.some((issue) => issue.includes("missing or empty"))).toBe(true);
  });

  it("passes every real permission catalog", () => {
    const spies = ["log", "error"].map((method) =>
      vi.spyOn(console, method).mockImplementation(() => {}),
    );
    try {
      expect(runPermissionSurfaceLocaleCheck()).toBe(0);
    } finally {
      spies.forEach((spy) => spy.mockRestore());
    }
  });
});
