import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { GroundingProvenance } from "./GroundingProvenance";

describe("GroundingProvenance", () => {
  it("shows a human access time while preserving the exact machine value", () => {
    const accessedAtUtc = "2026-07-23T14:12:13.456Z";
    render(
      <GroundingProvenance sources={[{
        url: "https://www.eia.gov/petroleum/gasdiesel/",
        accessedAtUtc,
      }]} />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("link", {
      name: "https://www.eia.gov/petroleum/gasdiesel/",
    })).toHaveAttribute("href", "https://www.eia.gov/petroleum/gasdiesel/");
    const accessed = screen.getByText(/^Accessed:/);
    expect(accessed).toHaveAttribute("datetime", accessedAtUtc);
    expect(accessed).toHaveTextContent(
      `Accessed: ${new Intl.DateTimeFormat("en-US", {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(accessedAtUtc))}`,
    );
    expect(accessed).not.toHaveTextContent("2026-07-23T14:12:13.456Z");
  });
});
