import { describe, expect, it } from "vitest";
import {
  redactSensitiveText,
  redactSensitiveValue,
  safeErrorMessage,
} from "../redaction";

describe("structured redaction", () => {
  it("redacts nested keys, arrays, error causes, and non-enumerable messages", () => {
    const cause = new Error(
      "request https://api.telegram.org/bot123456:telegram-canary/getUpdates?token=query-canary",
    );
    Object.assign(cause, { Authorization: "Bearer header-canary" });
    const error = new Error("password=message-canary", { cause });
    Object.assign(error, {
      nested: [{ api_key: "key-canary", safe: "visible" }],
      Cookie: "cookie-canary",
    });

    const serialized = JSON.stringify(redactSensitiveValue(error));
    for (const canary of [
      "telegram-canary",
      "query-canary",
      "header-canary",
      "message-canary",
      "key-canary",
      "cookie-canary",
    ]) {
      expect(serialized).not.toContain(canary);
    }
    expect(serialized).toContain("visible");
  });

  it("redacts credentials embedded in free text and URL userinfo", () => {
    const redacted = redactSensitiveText(
      "Authorization: Bearer header-canary\nCookie: session=cookie-canary; other=second-cookie-canary\nBasic abc123 https://user:pass@example.test/path?client_secret=url-canary secret=text-canary",
    );
    expect(redacted).not.toContain("header-canary");
    expect(redacted).not.toContain("cookie-canary");
    expect(redacted).not.toContain("second-cookie-canary");
    expect(redacted).not.toContain("abc123");
    expect(redacted).not.toContain("pass");
    expect(redacted).not.toContain("url-canary");
    expect(redacted).not.toContain("text-canary");
  });

  it("redacts space-separated CLI credential flags", () => {
    const redacted = redactSensitiveText(
      "helper --api-key cli-key-canary --token 'cli-token-canary' --password=cli-password-canary --safe visible",
    );
    expect(redacted).not.toContain("cli-key-canary");
    expect(redacted).not.toContain("cli-token-canary");
    expect(redacted).not.toContain("cli-password-canary");
    expect(redacted).toContain("--safe visible");
  });

  it("returns a safe message without preserving raw error fields", () => {
    expect(safeErrorMessage({ message: "token=canary", privateKey: "raw" }))
      .toBe("token=[redacted]");
  });

  it("removes usernames from POSIX and Windows home paths", () => {
    const redacted = redactSensitiveText(
      "at /Users/alice/private.ts:1 and C:\\Users\\alice\\private.ts:2",
    );
    expect(redacted).not.toContain("alice");
    expect(redacted.match(/\[home\]/g)).toHaveLength(2);
  });

  it("bounds wide structures and oversized chained error text", () => {
    const wide = Array.from({ length: 10_000 }, (_, index) => ({
      index,
      token: `canary-${index}`,
    }));
    const redacted = redactSensitiveValue({ wide });
    const serialized = JSON.stringify(redacted);
    expect(serialized).toContain("redacted-structure-limit");
    expect(serialized).not.toContain("canary-0");

    const message = safeErrorMessage(new Error(`token=message-canary ${"🙂".repeat(10_000)}`));
    expect(message.length).toBeLessThanOrEqual(4096);
    expect(message).not.toContain("message-canary");
    expect(message.endsWith("...[truncated]")).toBe(true);
  });
});
