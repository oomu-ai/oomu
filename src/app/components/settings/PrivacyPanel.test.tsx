import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { humanizeStorageBackend, PrivacyPanel } from "./PrivacyPanel";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("./TrustPanel", () => ({ TrustPanel: () => null }));

const recoveredProfile = {
  fingerprint: "1234567890abcdef1234567890abcdef",
  public_key: "ab".repeat(32),
  hardware_binding: "This Mac",
  storage_backend: "OS keychain",
};

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [
      {
        id: "en-US",
        label: "English (US)",
        fileName: "en-US.json",
        isDefault: true,
        verified: true,
      },
    ],
    translations: {},
  };
}

describe("PrivacyPanel device identity recovery", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_privacy_settings") {
        return { automatedWebGroundingEnabled: false };
      }
      if (command === "get_sovereign_identity") {
        throw { message: "ledger_integrity_violation" };
      }
      if (command === "retry_sovereign_identity_health") return recoveredProfile;
      return null;
    });
  });

  afterEach(cleanup);

  it("keeps the repair contextual and retries only when the user asks", async () => {
    render(<PrivacyPanel />, { wrapper: I18nProvider });

    expect(await screen.findByText("Device identity needs attention")).toBeVisible();
    expect(
      screen.getByText(
        "Chat and your saved work remain available. Try the secure identity check again before signing or verifying work.",
      ),
    ).toBeVisible();
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "get_sovereign_identity"),
    ).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Check again" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("retry_sovereign_identity_health"),
    );
    expect(await screen.findByText("1234567890abcdef...")).toBeVisible();
  });
});

describe("humanizeStorageBackend", () => {
  it("replaces database implementation names with the trusted local-storage label", () => {
    const fallback = "Encrypted on this Mac";
    expect(humanizeStorageBackend(null, fallback)).toBe(fallback);
    expect(humanizeStorageBackend("Encrypted SQLite (SQLCipher)", fallback)).toBe(fallback);
    expect(humanizeStorageBackend("Encrypted local vault", fallback)).toBe(
      "Encrypted local vault",
    );
  });
});
