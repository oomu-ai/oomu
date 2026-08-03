import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useRecommendedModelSettingsRoute } from "./recommendedModelSettingsRoute";
import { RecommendedModelInstallIndicator } from "./RecommendedModelInstallIndicator";
import type { RecommendedModelInstallProgress } from "./useRecommendedLocalModelInstall";

const installMock = vi.hoisted(() => ({
  progress: null as RecommendedModelInstallProgress | null,
}));

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    language: "en-US",
    t: (key: string, values?: Record<string, string | number>) => ({
      "recommended_model.downloading": "Downloading",
      "recommended_model.verifying": "Verifying download…",
      "recommended_model.preparing": "Preparing OOMU…",
      "recommended_model.download_progress": `${values?.downloaded} GB of ${values?.total} GB`,
      "recommended_model.download_progress_label": "Model download progress",
      "settings.tabs.models": "Models",
    })[key] ?? key,
  }),
}));

vi.mock("./useRecommendedLocalModelInstall", () => ({
  useRecommendedLocalModelInstall: () => ({ progress: installMock.progress }),
}));

const progress = (state: RecommendedModelInstallProgress["state"], downloadedBytes = 0) => ({
  installId: "install-1",
  state,
  downloadedBytes,
  totalBytes: 4_000_000_000,
  currentAsset: null,
  canCancel: state === "downloading",
  canResume: false,
  publicErrorCode: null,
  completedProvider: null,
} satisfies RecommendedModelInstallProgress);

function RouteListener({ onOpen }: { onOpen: () => void }) {
  useRecommendedModelSettingsRoute(onOpen);
  return null;
}

afterEach(cleanup);

describe("RecommendedModelInstallIndicator", () => {
  it("shows native byte progress after setup is deferred and opens the model destination", () => {
    installMock.progress = progress("downloading", 1_000_000_000);
    const openRoute = vi.fn();
    const openModels = vi.fn();
    render(<><RouteListener onOpen={openRoute} /><RecommendedModelInstallIndicator onOpenModels={openModels} /></>);

    const indicator = screen.getByRole("button", { name: "Downloading 25%. Models" });
    expect(indicator).toHaveAttribute("title", "1 GB of 4 GB");
    expect(screen.getByRole("progressbar", { name: "Model download progress" }))
      .toHaveAttribute("aria-valuenow", "25");
    fireEvent.click(indicator);
    expect(openRoute).toHaveBeenCalledOnce();
    expect(openModels).toHaveBeenCalledOnce();
  });

  it("stays visible for post-download verification without inventing percentage progress", () => {
    installMock.progress = progress("verifying", 4_000_000_000);
    render(<RecommendedModelInstallIndicator onOpenModels={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Verifying download…. Models" })).toBeVisible();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it.each(["idle", "cancelled", "ready", "failed"] as const)(
    "does not compete with the shell in the %s state",
    (state) => {
      installMock.progress = progress(state);
      render(<RecommendedModelInstallIndicator onOpenModels={vi.fn()} />);
      expect(screen.queryByRole("button")).toBeNull();
    },
  );
});
