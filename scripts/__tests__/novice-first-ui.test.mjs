import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { inspectNoviceFirstUi } from "../check-novice-first-ui.mjs";

const root = path.resolve(import.meta.dirname, "../..");
const temporary = [];

function copyGovernedTree() {
  const target = mkdtempSync(path.join(tmpdir(), "oomu-novice-ui-"));
  temporary.push(target);
  for (const relative of [
    "src/locales/en-US.json",
    "src/app/components/artifacts/ArtifactStudio.tsx",
    "src/app/components/artifacts/review/DocumentReviewShell.tsx",
    "src/app/components/artifacts/review/CreateDocumentAction.tsx",
    "src/app/components/artifacts/workbooks/WorkbookDocumentReview.tsx",
    "src/app/components/artifacts/presentations/PresentationDocumentReview.tsx",
    "src/app/components/computer_use/AppControlMonitor.tsx",
    "src/app/components/browser_automation/BrowserTaskPanel.tsx",
    "src/app/components/integrations/microsoft365/Microsoft365ControlPanel.tsx",
    "src/app/components/integrations/IntegrationsScreen.tsx",
    "src/app/components/integrations/SetupJourney.tsx",
    "src/app/components/tasks/EvidenceTimeline.tsx",
    "src/app/components/chat/ShieldApprovalDialog.tsx",
    "src/app/components/settings/RemoteDevicesPanel.tsx",
    "src/app/components/media/MediaTaskPanel.tsx",
    "src/app/components/ModsScreen.tsx",
    "src/app/components/delegation/ChildWorkstreams.tsx",
    "src/app/components/analysis/AnalysisResults.tsx",
    "src/app/components/learning/LearningReview.tsx",
  ]) {
    const destination = path.join(target, relative);
    mkdirSync(path.dirname(destination), { recursive: true });
    writeFileSync(destination, readFileSync(path.join(root, relative)));
  }
  return target;
}

afterEach(() => { while (temporary.length) rmSync(temporary.pop(), { recursive: true, force: true }); });

describe("novice-first UI gate", () => {
  it("accepts the governed Documents and Microsoft surfaces", () => {
    expect(inspectNoviceFirstUi(root)).toEqual([]);
  });

  it("rejects a raw status and a banned user-facing noun", () => {
    const target = copyGovernedTree();
    const localePath = path.join(target, "src/locales/en-US.json");
    const locale = JSON.parse(readFileSync(localePath, "utf8"));
    locale.documents.title = "Artifacts";
    writeFileSync(localePath, JSON.stringify(locale));
    const sourcePath = path.join(target, "src/app/components/artifacts/ArtifactStudio.tsx");
    writeFileSync(sourcePath, `${readFileSync(sourcePath, "utf8")}\nconst leak = health.detail;\n`);
    expect(inspectNoviceFirstUi(target).join("\n")).toMatch(/localized glass copy|technical detail/);
  });

  it("rejects artifact-engine copy that belongs behind Details", () => {
    const target = copyGovernedTree();
    const localePath = path.join(target, "src/locales/en-US.json");
    const locale = JSON.parse(readFileSync(localePath, "utf8"));
    locale.documents.subtitle = "Build from a master placeholder";
    writeFileSync(localePath, JSON.stringify(locale));
    expect(inspectNoviceFirstUi(target).join("\n")).toMatch(/localized glass copy/);
  });
});
