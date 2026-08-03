import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ContextCondensationNotice } from "./ContextCondensationNotice";

describe("ContextCondensationNotice", () => {
  afterEach(cleanup);

  it("reassures the user without exposing an internal token count", () => {
    render(
      <ContextCondensationNotice metadata={{
        contextCondensed: true,
        contextBudgetTokens: 12_288,
      }} />,
      { wrapper: I18nProvider },
    );

    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent(
      "To keep this chat fast, OOMU shortened some older background details. Nothing you asked about was lost.",
    );
    expect(notice).not.toHaveTextContent("12");
  });

  it("stays hidden when no condensation occurred", () => {
    render(
      <ContextCondensationNotice metadata={{ contextCondensed: false }} />,
      { wrapper: I18nProvider },
    );

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});
