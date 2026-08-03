import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

function shippedTsxFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return entry.name === "__tests__" ? [] : shippedTsxFiles(path);
    }
    return entry.isFile() && entry.name.endsWith(".tsx") && !entry.name.endsWith(".test.tsx")
      ? [path]
      : [];
  });
}

describe("button interaction contract", () => {
  it("gives every button the same hover, press, disabled, busy, success, attention, and error states", () => {
    const css = readFileSync(join(process.cwd(), "src/app/globals.css"), "utf8");

    for (const selector of [
      "button:not(:disabled):hover",
      "button:not(:disabled):active",
      "button:disabled",
      'button[aria-busy="true"]',
      'button[data-action-state="success"]',
      'button[data-action-state="attention"]',
      'button[data-action-state="error"]',
    ]) {
      expect(css, `missing global interaction state ${selector}`).toContain(selector);
    }
    expect(css).toContain("prefers-reduced-motion: reduce");
  });

  it("keeps shipped button colors on the shared OOMU token palette", () => {
    const forbiddenPalette =
      /(?:bg|border|text)-(?:amber|blue|cyan|emerald|gray|green|indigo|lime|neutral|orange|pink|purple|red|rose|slate|stone|teal|violet|yellow|zinc)-\d{2,3}|#[0-9a-f]{3,8}/iu;

    for (const file of [
      ...shippedTsxFiles(join(process.cwd(), "src/app")),
      ...shippedTsxFiles(join(process.cwd(), "src/components")),
    ]) {
      const source = readFileSync(file, "utf8");
      for (const button of source.matchAll(/<button\b[\s\S]*?>/gu)) {
        expect(button[0], `${file} contains a button outside the shared color tokens`).not.toMatch(
          forbiddenPalette,
        );
      }
    }
  });
});
