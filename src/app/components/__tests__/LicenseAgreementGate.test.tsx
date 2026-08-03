import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { PrivacySettingsState } from "@/lib/privacySettings";
import { LicenseAgreementGate } from "../HomeChrome";

const BASE_SETTINGS: PrivacySettingsState = {
  automatedWebGroundingEnabled: false,
  licenseAccepted: false,
  licenseState: "presented",
  acceptedLicenseVersion: null,
  acceptanceTimestampMs: null,
  licenseVersion: "1.2",
  licenseEffectiveDate: "July 10, 2026",
  licenseText: [
    "# COMPLETE LICENSE CANARY",
    "",
    "## Acceptance",
    "",
    "All **operative terms**.",
    "",
    "- Keep the complete license",
    "- Render it as readable text",
  ].join("\n"),
};

afterEach(cleanup);

describe("LicenseAgreementGate", () => {
  it("shows only the complete license and local accept or decline actions", () => {
    const onAccept = vi.fn();
    const onDecline = vi.fn();
    render(
      <LicenseAgreementGate
        onAccept={onAccept}
        onDecline={onDecline}
        settings={BASE_SETTINGS}
      />,
      { wrapper: I18nProvider },
    );

    const dialog = screen.getByRole("dialog", {
      name: "Review the OOMU license",
    });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByRole("heading", { name: "COMPLETE LICENSE CANARY" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Acceptance" })).toBeVisible();
    expect(screen.getByText("operative terms", { selector: "strong" })).toBeVisible();
    expect(screen.getByRole("list")).toBeVisible();
    expect(screen.queryByRole("button", { name: /learn more/i })).not.toBeInTheDocument();

    const accept = screen.getByRole("button", { name: "Accept License" });
    expect(accept).toHaveFocus();
    fireEvent.click(accept);
    fireEvent.click(screen.getByRole("button", { name: "Decline and Quit" }));
    expect(onAccept).toHaveBeenCalledTimes(1);
    expect(onDecline).toHaveBeenCalledTimes(1);
  });

  it("disables both decisions while acceptance is being saved", () => {
    render(
      <LicenseAgreementGate
        isAccepting
        onAccept={vi.fn()}
        onDecline={vi.fn()}
        settings={BASE_SETTINGS}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("button", { name: "Saving Acceptance…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Decline and Quit" })).toBeDisabled();
  });
});
