#!/usr/bin/env node

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const governedSources = [
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
];

const bannedCopy = [
  /\bartifacts?\b/i,
  /\bdeliverables?\b/i,
  /\bOOXML\b/i,
  /\bcalculated-vs-stale\b/i,
  /\bsource lineage\b/i,
  /\bformula cells?\b/i,
  /\btemplate identity\b/i,
  /\bprovenance anchors?\b/i,
  /\bbuilderIdentity\b/,
  /\brendererIdentity\b/,
];
const artifactSurfaceBannedCopy = [
  /\bbuild\b/i,
  /\bmasters?\b/i,
  /\bplaceholders?\b/i,
];
const appControlBannedCopy = [
  /\bobservation(?: revision)?\b/i,
  /\belement reference\b/i,
  /\bpending mutation\b/i,
  /\bpostcondition\b/i,
  /\baccessibility tree\b/i,
  /\bbundle id\b/i,
  /\bapple events\b/i,
  /\bactuation\b/i,
  /\bsemantic snapshot\b/i,
];

function strings(value, prefix = "") {
  if (typeof value === "string") return [{ path: prefix, value }];
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  return Object.entries(value).flatMap(([key, item]) => strings(item, prefix ? `${prefix}.${key}` : key));
}

export function inspectNoviceFirstUi(checkRoot = root) {
  const failures = [];
  for (const file of readdirSync(path.join(checkRoot, "src/locales")).filter((name) => name.endsWith(".json"))) {
    const locale = JSON.parse(readFileSync(path.join(checkRoot, "src/locales", file), "utf8"));
    for (const entry of strings(locale)) {
      const patterns = /^(?:documents|workbooks|workbook_labels|presentations|presentation_(?:renderer|template|checks|issues))\./.test(entry.path)
        ? [...bannedCopy, ...artifactSurfaceBannedCopy]
        : /^(?:app_control|app_control_actions|browser)\./.test(entry.path)
          ? [...bannedCopy, ...appControlBannedCopy]
        : bannedCopy;
      for (const pattern of patterns) {
        if (pattern.test(entry.value)) failures.push(`${file}: localized glass copy ${entry.path} contains ${pattern}`);
      }
    }
  }

  const source = Object.fromEntries(governedSources.map((relative) => [relative, readFileSync(path.join(checkRoot, relative), "utf8")]));
  const library = source["src/app/components/artifacts/ArtifactStudio.tsx"];
  if (/choose_task|taskRunId.*select|taskApi\.list/.test(library)) failures.push("Documents library must not require a Task selector");
  if (!library.includes("WorkbookDocumentReview") || !library.includes("DocumentReviewShell")) failures.push("Documents must normalize Word/PDF and spreadsheets into the shared shell");
  const shell = source["src/app/components/artifacts/review/DocumentReviewShell.tsx"];
  if (!shell.includes("<details") || shell.indexOf("{preview}") > shell.indexOf("<details")) failures.push("shared review shell must be preview-first with progressive Details");
  const createDocument = source["src/app/components/artifacts/review/CreateDocumentAction.tsx"];
  if (!createDocument.includes("documentCreationApi.createFromTask")) failures.push("contextual Task creation must use the reusable creation API");
  if (!createDocument.includes("presentationApi.inspectTemplate") || !createDocument.includes("taskSummaryCompatible")) failures.push("presentation template choice must use the native picker and fail closed on incompatible designs");
  const presentations = source["src/app/components/artifacts/presentations/PresentationDocumentReview.tsx"];
  if (!presentations.includes("presentationApi.preview") || !presentations.includes("filmstrip") || !presentations.includes("revisionHistory")) failures.push("presentation review must load filmstrip previews and retain version history");
  if (!presentations.includes("<details") || !presentations.includes("ISSUE_KEYS") || !presentations.includes("CHECK_KEYS")) failures.push("presentation checks and sources must use progressive disclosure and localized label maps");
  if (!presentations.includes("targetObjectIds") && !presentations.includes("changedObjectIds")) failures.push("single presentation edits must use element-scoped revisions");
  if (!presentations.includes('"narrative_section"')) failures.push("multi-field story edits must remain scoped to their selected narrative section");
  if (!presentations.includes("elementFrameStyle") || !presentations.includes("exact_package_pages_rendered")) failures.push("presentation issues and exact-package evidence must remain visible through plain localized mappings");
  if (/\{(?:issue\.message|check\.detail)\}/.test(presentations)) failures.push("presentation review must not render backend issue or check prose");
  const appControl = source["src/app/components/computer_use/AppControlMonitor.tsx"];
  if (!appControl.includes("ACTION_KEYS") || !appControl.includes("PAUSE_KEYS") || !appControl.includes("OUTCOME_KEYS")) failures.push("app control must map every engine state through localized labels");
  if (!appControl.includes('onControl("take_control")') || !appControl.includes('onControl("return_to_oomu")') || !appControl.includes("app_control.pause")) failures.push("app control must keep pause, takeover, and handback one click away");
  if (/\{session\.(?:sessionId|taskRunId|projectId|observationGeneration)\}/.test(appControl)) failures.push("app control must not render engine identifiers");
  const browser = source["src/app/components/browser_automation/BrowserTaskPanel.tsx"];
  if (!browser.includes("browser.take_control") || !browser.includes("browser.return_to_oomu") || /expectedPostcondition|elementReference/.test(browser)) failures.push("browser supervision must use the same ambient takeover pattern without engine controls");

  for (const [relative, text] of Object.entries(source)) {
    for (const pattern of [/\.builderIdentity\b/, /\.rendererIdentity\b/, /health\.detail\b/, /health\.repairAction\b/, /JSON\.stringify\(/]) {
      if (pattern.test(text)) failures.push(`${relative} leaks governed backend or technical detail (${pattern})`);
    }
  }

  const microsoft = source["src/app/components/integrations/microsoft365/Microsoft365ControlPanel.tsx"];
  if (!microsoft.includes("ApprovalDialogFrame") || !microsoft.includes("requiredScopes") || !microsoft.includes("requestedOperations: [review.operation]")) failures.push("Microsoft consent must review one exact capability before OAuth");
  if (!microsoft.includes("<details")) failures.push("Microsoft technical account evidence must be progressively disclosed");
  if (!microsoft.includes("scopePurposes") || !microsoft.includes("destinationPurposes") || microsoft.includes("microsoft365_labels.technical_details") || /review\.scopes\.map\(|destinations\.map\(\(destination\).*destination\}/s.test(microsoft)) failures.push("Microsoft consent must show only plain-language access and service purposes");
  const integrations = source["src/app/components/integrations/IntegrationsScreen.tsx"];
  if (!/microsoftSelected\s*\?/.test(integrations) || !integrations.includes("<Microsoft365ControlPanel manifest={selected}")) failures.push("Microsoft must use its dedicated consent-governed service body");
  if (/JSON\.stringify|item\.detail|item\.repairAction|>\{manifest\.transport\}/.test(integrations)) failures.push("Integrations must not expose raw transport, health prose, or schemas");
  const setup = source["src/app/components/integrations/SetupJourney.tsx"];
  if (!setup.includes('item.manifestId === "microsoft_365"') || /connect\(["']microsoft_365["']\)/.test(setup)) failures.push("Setup must not bypass Microsoft exact-consent review");
  const evidence = source["src/app/components/tasks/EvidenceTimeline.tsx"];
  if (!evidence.includes("evidence.classes") || /Signed artifact|Model assertion|Executed mutation/.test(evidence)) failures.push("evidence classes must cross the localized label boundary");
  const shield = source["src/app/components/chat/ShieldApprovalDialog.tsx"];
  if (!shield.includes('request.actionType === "artifact_export"') || !shield.includes('request.actionType === "workbook_export"') || !shield.includes("knownExport")) failures.push("document exports must cross the localized Shield label boundary");
  if (!shield.includes('request.actionType === "presentation_export"') || !shield.includes('request.actionType === "app_control"') || !shield.includes("safeAppControlPreview")) failures.push("presentation export and app control must cross the localized Shield label boundary");
  if (!shield.includes('request.actionType === "connector_transmission"') || !shield.includes("safeTransmissionPreview")) failures.push("connected-data transmission consent must cross the localized Shield label boundary");
  const devices = source["src/app/components/settings/RemoteDevicesPanel.tsx"];
  if (/challenge\.qrSvg|create_remote_pairing_challenge|confirm_remote_pairing/.test(devices)) failures.push("remote settings must not advertise a pairing flow without a usable companion transport");
  if (!devices.includes("remote_devices.unavailable_title") || !devices.includes("remote_devices.unavailable_help")) failures.push("unavailable remote pairing must be explained honestly in plain language");
  if (/nonce|public key|revision number|step-up auth/i.test(devices)) failures.push("remote device glass copy must not expose protocol jargon");
  const media = source["src/app/components/media/MediaTaskPanel.tsx"];
  if (!media.includes("navigator.mediaDevices.getUserMedia") || !media.includes("microphone_off") || !media.includes("speechSynthesis.cancel")) failures.push("media review must make capture off-by-default and speech interruption explicit");
  const mods = source["src/app/components/ModsScreen.tsx"];
  if (
    !mods.includes("reviewState") ||
    !mods.includes("BUNDLE_CAPABILITY_SENTENCE_KEYS") ||
    !mods.includes("bundleCapabilitySentenceKey") ||
    !mods.includes('"mods.capability_sentences.other"') ||
    !mods.includes("acknowledgeUnreviewed")
  ) failures.push("bundle installation must separate review from publisher identity and use consequence sentences");
  if (/manifest JSON|permission.scope|supply chain|quarantine/i.test(mods)) failures.push("bundle review must not expose package-manager jargon");
  const helpers = source["src/app/components/delegation/ChildWorkstreams.tsx"];
  if (!helpers.includes("helpers.findings") || !helpers.includes("isDeveloperBuild")) failures.push("helpers must lead with findings and hide engine detail outside developer builds");
  if (/source\.digest|modelRoute|maxInputTokens|token budget/.test(helpers)) failures.push("helpers must not expose hashes, model routes, or token accounting in the default view");
  const analysis = source["src/app/components/analysis/AnalysisResults.tsx"];
  if (!analysis.includes("analysis.show_work") || !analysis.includes("isDeveloperBuild")) failures.push("analysis must lead with the answer and progressively disclose its work");
  const learning = source["src/app/components/learning/LearningReview.tsx"];
  if (!learning.includes("learning.offer_title") || !learning.includes("remember_everywhere") || !learning.includes("ApprovalDialogFrame")) failures.push("learning must be framed as a reviewable offer with explicit everywhere confirmation");
  if (learning.includes("window.confirm")) failures.push("learning consent must use the shared, localized permission dialog instead of a browser confirmation");
  return failures;
}

export function runNoviceFirstUiCheck(checkRoot = root) {
  const failures = inspectNoviceFirstUi(checkRoot);
  if (failures.length) {
    console.error("novice-first-ui: FAIL");
    failures.forEach((failure) => console.error(`- ${failure}`));
    return 1;
  }
  console.log("novice-first-ui: PASS (purpose, readiness, next action, progressive disclosure, and label boundaries verified)");
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(runNoviceFirstUiCheck());
}
