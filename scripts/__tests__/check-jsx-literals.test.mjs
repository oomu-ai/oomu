import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DENYLIST,
  DENYLIST_PATTERNS,
  ENFORCED_FILES,
  denialReason,
  discoverScannedFiles,
  findLiteralsInSource,
  runJsxLiteralCheck,
} from "../check-jsx-literals.mjs";

const root = path.resolve(import.meta.dirname, "../..");
const heroFixture = path.join(
  import.meta.dirname,
  "fixtures/decision-brief-pre-i18n.tsx.fixture",
);
const literalFixture = (name) => path.join(import.meta.dirname, "fixtures", name);
const temporaryRoots = [];

afterEach(() => {
  while (temporaryRoots.length > 0) {
    rmSync(temporaryRoots.pop(), { force: true, recursive: true });
  }
});

function quietConsole() {
  return ["log", "error", "warn"].map((method) =>
    vi.spyOn(console, method).mockImplementation(() => {}),
  );
}

// Locks the JSX-literal guard: the audit history shows new surfaces shipping
// hardcoded English (DegradedModeLanding, UserConfigPanel, the Memory popover).
// This guard is the other half of the English-first policy — components must
// route copy through t() so the batch translator can see it.
describe("JSX literal guard", () => {
  it("flags a hardcoded JSX text node", () => {
    const findings = findLiteralsInSource(`<button>Save Profile</button>`);
    expect(findings.some((finding) => finding.kind === "text" && finding.text === "Save Profile")).toBe(true);
  });

  it("flags a hardcoded English attribute", () => {
    const findings = findLiteralsInSource(`<input placeholder="What should OOMU call you?" />`);
    expect(findings.some((finding) => finding.kind === "attr")).toBe(true);
  });

  it("flags multiline JSX text from a valid component fixture", () => {
    const findings = findLiteralsInSource(
      readFileSync(literalFixture("multiline-jsx-text.tsx.fixture"), "utf8"),
    );
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "text", text: "Save Profile" }),
    ]));
  });

  it("flags a single-quoted JSX copy attribute from a valid component fixture", () => {
    const findings = findLiteralsInSource(
      readFileSync(literalFixture("single-quoted-jsx-attribute.tsx.fixture"), "utf8"),
    );
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "attr", text: 'aria-label="Save Profile"' }),
    ]));
  });

  it("flags a brace-wrapped JSX string", () => {
    const findings = findLiteralsInSource(`<p>{"Hero workflow"}</p>`);
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "expression", text: "Hero workflow" }),
    ]));
  });

  it("flags English copy nested in conditional and template JSX expressions", () => {
    const findings = findLiteralsInSource(
      readFileSync(literalFixture("nested-jsx-expression.tsx.fixture"), "utf8"),
    );
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "expression", text: "Open {value}" }),
      expect.objectContaining({ kind: "expression", text: "Navigation blocked" }),
      expect.objectContaining({ kind: "expression", text: "Confirm browser navigation" }),
    ]));
  });

  it("flags only English copy-object properties that render in JSX", () => {
    const findings = findLiteralsInSource(`
      const copy = { title: "Weekly Decision Brief", detail: "Shows what OOMU used.", id: "machine_id", unused: "Not rendered" };
      export function Screen() { return <><h1>{copy.title}</h1><p>{copy.detail}</p></>; }
    `);
    expect(findings.filter((finding) => finding.kind === "copy-object").map((finding) => finding.text)).toEqual([
      "copy.title: Weekly Decision Brief",
      "copy.detail: Shows what OOMU used.",
    ]);
  });

  it("flags a rendered local copy object wrapped with as const", () => {
    const findings = findLiteralsInSource(
      readFileSync(literalFixture("as-const-copy-object.tsx.fixture"), "utf8"),
    );
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "copy-object",
        text: "copy.title: Weekly Decision Brief",
      }),
    ]));
  });

  it("flags a rendered local copy object wrapped with satisfies", () => {
    const findings = findLiteralsInSource(
      readFileSync(literalFixture("satisfies-copy-object.tsx.fixture"), "utf8"),
    );
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "copy-object",
        text: "copy.detail: Shows what OOMU used.",
      }),
    ]));
  });

  it("does not flag an English object that is not rendered", () => {
    expect(findLiteralsInSource(`
      const fallbacks = { title: "Intentional test fallback" };
      export function Screen() { return <h1>{t("screen.title")}</h1>; }
    `)).toEqual([]);
  });

  it("passes text routed through t()", () => {
    expect(findLiteralsInSource(`<button>{t("user_config.save")}</button>`)).toEqual([]);
  });

  it("still flags hardcoded copy on a line that also contains a translated call", () => {
    const findings = findLiteralsInSource(`<button>Hard coded text</button>{t("user_config.save")}`);
    expect(findings.some((finding) => finding.text === "Hard coded text")).toBe(true);
  });

  it("flags single-word user copy", () => {
    const findings = findLiteralsInSource(`<button>Export</button>`);
    expect(findings.some((finding) => finding.text === "Export")).toBe(true);
  });

  it("ignores SVG geometry and aria-hidden decoration", () => {
    expect(
      findLiteralsInSource(`<path d="M6 6l12 12" /><span aria-hidden="true">!</span>`),
    ).toEqual([]);
  });

  it("catches the permanent pre-i18n Decision Brief snapshot", () => {
    const findings = findLiteralsInSource(readFileSync(heroFixture, "utf8"));
    const renderedCopy = findings.filter((finding) => finding.kind === "copy-object");
    expect(findings.length).toBeGreaterThanOrEqual(5);
    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "expression", text: "Hero workflow" }),
    ]));
    expect(renderedCopy.length).toBeGreaterThanOrEqual(3);
    expect(renderedCopy.map((finding) => finding.text).join("\n")).toContain("Weekly Decision Brief");
    expect(renderedCopy.map((finding) => finding.text).join("\n")).toContain("evidence-bound path");
    expect(renderedCopy.map((finding) => finding.text).join("\n")).toContain("Results appear first");
  });

  it("keeps the established clean surfaces free of false positives", () => {
    for (const relative of [
      "src/app/components/TrustSummary.tsx",
      "src/app/components/computer_use/AppControlMonitor.tsx",
      "src/app/components/ChannelsDashboard.tsx",
    ]) {
      expect(findLiteralsInSource(readFileSync(path.join(root, relative), "utf8")), relative).toEqual([]);
    }
  });

  it("discovers a brand-new component and fails it without a gate edit", () => {
    const target = mkdtempSync(path.join(tmpdir(), "oomu-jsx-literal-"));
    temporaryRoots.push(target);
    const component = path.join(target, "src/app/components/NewScreen.tsx");
    mkdirSync(path.dirname(component), { recursive: true });
    writeFileSync(component, `export function NewScreen() { return <button>Brand new copy</button>; }`);

    expect(discoverScannedFiles(target)).toEqual(["src/app/components/NewScreen.tsx"]);
    const spies = quietConsole();
    try {
      expect(runJsxLiteralCheck({ root: target })).toBe(1);
    } finally {
      spies.forEach((spy) => spy.mockRestore());
    }
  });

  it("excludes only reason-bearing exact and test-pattern deny rules", () => {
    for (const [relative, reason] of Object.entries(DENYLIST)) {
      expect(reason.trim(), relative).not.toBe("");
      expect(reason, relative).not.toContain("\n");
      expect(denialReason(relative), relative).toBe(reason);
    }
    for (const { pattern, reason } of DENYLIST_PATTERNS) {
      expect(pattern).toBeInstanceOf(RegExp);
      expect(reason.trim()).not.toBe("");
      expect(reason).not.toContain("\n");
    }
    expect(denialReason("src/app/components/Example.test.tsx")).toBe("test-only TSX file");
    expect(denialReason("src/app/components/__tests__/Example.tsx")).toBe("test-only fixture or harness under __tests__");
    expect(denialReason("src/app/components/Example.tsx")).toBeNull();
  });

  it("governs all current non-denied product components by discovery", () => {
    expect(ENFORCED_FILES).toEqual(discoverScannedFiles(root));
    expect(ENFORCED_FILES).toEqual(expect.arrayContaining([
      "src/app/components/hero/DecisionBriefScreen.tsx",
      "src/app/components/routines/RoutinesScreen.tsx",
      "src/app/components/tasks/TaskCenter.tsx",
      "src/app/components/projects/ProjectsScreen.tsx",
      "src/app/components/artifacts/ArtifactStudio.tsx",
      "src/app/components/integrations/IntegrationsScreen.tsx",
      "src/app/components/integrations/microsoft365/Microsoft365ControlPanel.tsx",
    ]));
    expect(ENFORCED_FILES).not.toEqual(expect.arrayContaining(Object.keys(DENYLIST)));
  });
});
