import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider, useI18n } from "../I18nContext";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

function LocaleConsumer() {
  const { availableLocales, language, setLanguage, t } = useI18n();
  return (
    <div>
      <div data-testid="language">{language}</div>
      <div data-testid="locale-ids">
        {availableLocales.map((locale) => locale.id).join(",")}
      </div>
      <div data-testid="settings-title">{t("settings.title")}</div>
      <div data-testid="knowledge-help">{t("projects.knowledge_help")}</div>
      <div data-testid="execution-error">
        {t("chat.errors.final_verification.content")}
      </div>
      <button type="button" onClick={() => void setLanguage("de-DE")}>
        Use German
      </button>
    </div>
  );
}

describe("I18nProvider", () => {
  const originalDocumentLanguage = document.documentElement.getAttribute("lang");

  afterEach(() => {
    cleanup();
    invokeMock.mockReset();
    if (originalDocumentLanguage === null) {
      document.documentElement.removeAttribute("lang");
    } else {
      document.documentElement.setAttribute("lang", originalDocumentLanguage);
    }
  });

  it("sets the document language to the default locale on initial load", () => {
    invokeMock.mockReturnValue(new Promise(() => {}));

    render(
      <I18nProvider>
        <LocaleConsumer />
      </I18nProvider>,
    );

    expect(document.documentElement).toHaveAttribute("lang", "en-US");
  });

  it("exposes every verified locale returned by the backend", async () => {
    invokeMock.mockResolvedValue({
      activeLocale: "es-ES",
      availableLocales: [
        {
          id: "en-US",
          label: "English (US)",
          fileName: "en-US.json",
          isDefault: true,
          verified: true,
        },
        {
          id: "es-ES",
          label: "Spanish",
          fileName: "es-ES.json",
          isDefault: false,
          verified: true,
        },
        {
          id: "de-DE",
          label: "German",
          fileName: "de-DE.json",
          isDefault: false,
          verified: true,
        },
      ],
      translations: {
        settings: {
          title: "Configuracion",
        },
      },
    });

    render(
      <I18nProvider>
        <LocaleConsumer />
      </I18nProvider>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("language")).toHaveTextContent("es-ES");
    });
    expect(screen.getByTestId("locale-ids")).toHaveTextContent(
      "en-US,es-ES,de-DE",
    );
    expect(screen.getByTestId("settings-title")).toHaveTextContent(
      "Configuracion",
    );
    expect(screen.getByTestId("knowledge-help")).toHaveTextContent(
      "Add a folder once.",
    );
    expect(screen.getByTestId("execution-error")).toHaveTextContent(
      "OOMU couldn’t safely confirm the result",
    );
    expect(document.documentElement).toHaveAttribute("lang", "es-ES");
  });

  it("keeps the document language synchronized and restores it on unmount", async () => {
    document.documentElement.setAttribute("lang", "en");

    const localePayload = {
      activeLocale: "es-ES",
      availableLocales: [
        {
          id: "en-US",
          label: "English (US)",
          fileName: "en-US.json",
          isDefault: true,
          verified: true,
        },
        {
          id: "es-ES",
          label: "Spanish",
          fileName: "es-ES.json",
          isDefault: false,
          verified: true,
        },
        {
          id: "de-DE",
          label: "German",
          fileName: "de-DE.json",
          isDefault: false,
          verified: true,
        },
      ],
      translations: {},
    };

    invokeMock.mockImplementation((command: string) => {
      if (command === "set_active_locale") {
        return Promise.resolve({ ...localePayload, activeLocale: "de-DE" });
      }
      return Promise.resolve(localePayload);
    });

    const { unmount } = render(
      <I18nProvider>
        <LocaleConsumer />
      </I18nProvider>,
    );

    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("lang", "es-ES");
    });

    fireEvent.click(screen.getByRole("button", { name: "Use German" }));

    await waitFor(() => {
      expect(document.documentElement).toHaveAttribute("lang", "de-DE");
    });
    expect(screen.getByTestId("knowledge-help")).toHaveTextContent(
      "Add a folder once.",
    );
    expect(screen.getByTestId("execution-error")).toHaveTextContent(
      "OOMU couldn’t safely confirm the result",
    );

    unmount();
    expect(document.documentElement).toHaveAttribute("lang", "en");
  });
});
