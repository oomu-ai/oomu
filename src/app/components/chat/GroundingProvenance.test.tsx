import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { GroundingProvenance } from "./GroundingProvenance";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("GroundingProvenance", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined));
  afterEach(cleanup);

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

  it("opens one verified source click through the native HTTP browser boundary", () => {
    const url = "https://www.rust-lang.org/tools/install";
    render(
      <GroundingProvenance sources={[{
        url,
        accessedAtUtc: "2026-07-24T10:00:00.000Z",
      }]} />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("link", { name: url }));
    expect(invokeMock).toHaveBeenCalledWith("open_external_http_url", { url });
  });
});
