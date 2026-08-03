import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useResizablePanel } from "../ChatPanelResize";

const storedValues = new Map<string, string>();
const localStorageMock: Storage = {
  get length() {
    return storedValues.size;
  },
  clear() {
    storedValues.clear();
  },
  getItem(key: string) {
    return storedValues.get(key) ?? null;
  },
  key(index: number) {
    return Array.from(storedValues.keys())[index] ?? null;
  },
  removeItem(key: string) {
    storedValues.delete(key);
  },
  setItem(key: string, value: string) {
    storedValues.set(key, value);
  },
};

function PanelProbe({ storageKey = "oomu.test.panelWidth" }: { storageKey?: string }) {
  const panel = useResizablePanel({
    storageKey,
    defaultWidth: 256,
    min: 170,
    max: 420,
    side: "right",
  });

  return <div data-testid="panel-width">{panel.width}</div>;
}

beforeEach(() => {
  storedValues.clear();
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
});

afterEach(() => {
  cleanup();
  storedValues.clear();
});

describe("useResizablePanel", () => {
  it("renders the default width before browser storage is loaded", () => {
    window.localStorage.setItem("oomu.test.panelWidth", "216");

    expect(renderToString(<PanelProbe />)).toContain(">256<");
  });

  it("restores the persisted width after mount", async () => {
    window.localStorage.setItem("oomu.test.panelWidth", "216");

    render(<PanelProbe />);

    await waitFor(() => {
      expect(screen.getByTestId("panel-width")).toHaveTextContent("216");
    });
  });
});
