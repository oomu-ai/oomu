import { describe, expect, it } from "vitest";
import enUS from "@/locales/en-US.json";
import { projectSourceStateLabel } from "./ProjectsScreen";

const t = (key: string) => key;

describe("Project source labels", () => {
  it("maps backend source states to human status keys", () => {
    expect(projectSourceStateLabel(t, "ready")).toBe("projects.source_state_ready");
    expect(projectSourceStateLabel(t, "indexed")).toBe("projects.source_state_ready");
    expect(projectSourceStateLabel(t, "pending")).toBe("projects.source_state_pending");
    expect(projectSourceStateLabel(t, "indexing")).toBe("projects.source_state_indexing");
    expect(projectSourceStateLabel(t, "failed")).toBe("projects.source_state_attention");
    expect(projectSourceStateLabel(t, "future_state")).toBe("projects.source_state_unknown");
  });

  it("describes active file reading in the user's language", () => {
    expect(enUS.projects.source_state_indexing).toBe("Reading your files…");
  });
});
