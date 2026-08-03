import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

type JSDOMBackedGlobal = typeof globalThis & {
  jsdom?: { window: Window };
};

const jsdomLocalStorage = (globalThis as JSDOMBackedGlobal).jsdom?.window.localStorage;
if (jsdomLocalStorage) {
  // Node 25 exposes its own process-level localStorage accessor. Vitest does not
  // replace an existing global, so point browser tests at jsdom's real storage.
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: jsdomLocalStorage,
  });
}

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    addEventListener: vi.fn(),
    addListener: vi.fn(),
    dispatchEvent: vi.fn(),
    matches: false,
    media: query,
    onchange: null,
    removeEventListener: vi.fn(),
    removeListener: vi.fn(),
  })),
});

class ResizeObserverStub implements ResizeObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

vi.stubGlobal("ResizeObserver", ResizeObserverStub);
