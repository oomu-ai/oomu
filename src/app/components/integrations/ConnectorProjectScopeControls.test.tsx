import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ConnectorProjectScopeControls } from "./ConnectorProjectScopeControls";

const projects = [
  { projectId: "project-a", name: "Alpha" },
  { projectId: "project-b", name: "Beta" },
] as never[];

const account = {
  connectorId: "connector-a",
  connectionState: "authorized",
  allProjectsEnabled: false,
  projectScopeReviewedAtMs: 10,
  enabledProjectIds: ["project-a"],
};

afterEach(cleanup);

describe("ConnectorProjectScopeControls", () => {
  it("defaults a new authorized account to an unsaved all-projects draft", () => {
    render(<ConnectorProjectScopeControls
      account={{ ...account, projectScopeReviewedAtMs: null, enabledProjectIds: [] }}
      projects={[]}
      saveScope={vi.fn()}
    />, { wrapper: I18nProvider });
    expect(screen.getByRole("checkbox", { name: /Use in all my projects/ })).toBeChecked();
    expect(screen.getByText(/won’t use this connection until you save/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Save project access" })).toBeEnabled();
  });

  it("saves one atomic projection and restores native truth after failure", async () => {
    const saveScope = vi.fn().mockRejectedValue(new Error("offline"));
    render(<ConnectorProjectScopeControls account={account} projects={projects} saveScope={saveScope} />, { wrapper: I18nProvider });
    const beta = screen.getByRole("checkbox", { name: "Beta" });
    fireEvent.click(beta);
    fireEvent.click(screen.getByRole("button", { name: "Save project access" }));
    await waitFor(() => expect(saveScope).toHaveBeenCalledWith(false, ["project-a", "project-b"]));
    expect(await screen.findByRole("alert")).toHaveTextContent("Project access was not changed");
    expect(screen.getByRole("checkbox", { name: "Alpha" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Beta" })).not.toBeChecked();
  });

  it("retains the narrow checklist while all-projects is saved", async () => {
    const saveScope = vi.fn().mockResolvedValue({
      connectorId: "connector-a",
      allProjectsEnabled: true,
      enabledProjectIds: ["project-a"],
      projectScopeReviewedAtMs: 20,
      updatedAtMs: 20,
    });
    render(<ConnectorProjectScopeControls account={account} projects={projects} saveScope={saveScope} />, { wrapper: I18nProvider });
    fireEvent.click(screen.getByRole("checkbox", { name: /Use in all my projects/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save project access" }));
    await waitFor(() => expect(saveScope).toHaveBeenCalledWith(true, ["project-a"]));
    expect(await screen.findByText("Project access saved.")).toBeVisible();
  });
});
