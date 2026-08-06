import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("application update restart contract", () => {
  it("returns through Tauri's event loop before relaunching", () => {
    const source = readFileSync(
      join(process.cwd(), "src-tauri", "src", "app_updates.rs"),
      "utf8",
    );
    const start = source.indexOf("pub fn restart_after_application_update");
    const end = source.indexOf("\n#[cfg(test)]", start);
    const restartCommand = source.slice(start, end);

    expect(start).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(start);
    expect(restartCommand).toContain("app.request_restart();");
    expect(restartCommand).not.toContain("app.restart();");
  });
});
