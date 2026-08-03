import { describe, expect, it } from "vitest";
import { candidateLocalPathsFromText, parseLocalPathReferences } from "./localPathIntent";

const scenarioOnePrompt = "OOMU, prepare a board-ready supplier decision pack. Read `mock_data/supplier_proposals.json` and `mock_data/q3_strategic_vendor_proposals.txt` from my testing folder. Create a new `ship_test_01` folder there.";

describe("parseLocalPathReferences", () => {
  it("preserves both Scenario 1 Markdown paths as workspace-relative references", () => {
    const references = parseLocalPathReferences(scenarioOnePrompt);

    expect(references.map(({ normalizedText, kind, markdownWrapped }) => ({ normalizedText, kind, markdownWrapped }))).toEqual([
      { normalizedText: "mock_data/supplier_proposals.json", kind: "workspace_relative", markdownWrapped: true },
      { normalizedText: "mock_data/q3_strategic_vendor_proposals.txt", kind: "workspace_relative", markdownWrapped: true },
    ]);
    expect(candidateLocalPathsFromText(scenarioOnePrompt)).not.toContain("/supplier_proposals.json`");
  });

  it("keeps absolute, home, relative, and file URI references distinct", () => {
    const references = parseLocalPathReferences([
      "Read `/Users/example/My Files/report.md`,",
      "then '~/Documents/notes.md';",
      "compare ./draft.md and ../shared/input.json.",
      "Finally inspect file:///tmp/final%20report.md.",
    ].join(" "));

    expect(references.map(({ normalizedText, kind }) => ({ normalizedText, kind }))).toEqual([
      { normalizedText: "/Users/example/My Files/report.md", kind: "absolute" },
      { normalizedText: "~/Documents/notes.md", kind: "home_relative" },
      { normalizedText: "./draft.md", kind: "workspace_relative" },
      { normalizedText: "../shared/input.json", kind: "workspace_relative" },
      { normalizedText: "/tmp/final report.md", kind: "file_uri" },
    ]);
  });

  it("keeps a full absolute iCloud path intact inside a compound prompt", () => {
    const path = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Testing/Scenario 1/input.md";
    const prompt = `Compare \`${path}\` with mock_data/reference.md and recommend the next step without opening either path.`;

    expect(parseLocalPathReferences(prompt).map((reference) => reference.normalizedText)).toEqual([
      path,
      "mock_data/reference.md",
    ]);
  });

  it("keeps the unquoted Scenario 1 iCloud file path intact without inventing Documents authority", () => {
    const path = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mocked_data/supplier_proposals.json";
    const prompt = `prepare a board-ready supplier decision pack. Read ${path} and q3_strategic_vendor_proposals.txt from my testing folder. Reconcile every quoted amount and margin.`;

    expect(parseLocalPathReferences(prompt).map((reference) => reference.normalizedText)).toEqual([
      path,
    ]);
    expect(candidateLocalPathsFromText(prompt)).toEqual([path]);
  });

  it("preserves the escaped canonical iCloud identity in the published Scenario 3 summary", () => {
    const escapedPath = String.raw`/Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt`;
    const canonicalPath = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt";
    const prompt = `Read \`${escapedPath}\` in my testing folder and summarize only the stated facts in exactly three bullets.`;

    expect(parseLocalPathReferences(prompt).map(({ normalizedText, kind }) => ({ normalizedText, kind }))).toEqual([
      { normalizedText: canonicalPath, kind: "absolute" },
    ]);
  });

  it.each([
    [
      "List /Users/example/Library/Application Support/OOMU Data.",
      "/Users/example/Library/Application Support/OOMU Data",
    ],
    [
      "Inspect /Users/example/Projects/Supplier Review",
      "/Users/example/Projects/Supplier Review",
    ],
    [
      "Open /Users/example/Projects/Supplier Review, then summarize the folder.",
      "/Users/example/Projects/Supplier Review",
    ],
    [
      "Inspect /Users/example/Projects/Supplier Review/v1.2/Source Files",
      "/Users/example/Projects/Supplier Review/v1.2/Source Files",
    ],
  ])("recognizes an unquoted directory with spaces at a conservative boundary: %s", (prompt, path) => {
    expect(parseLocalPathReferences(prompt).map((reference) => reference.normalizedText)).toEqual([
      path,
    ]);
  });

  it("stops an unquoted file path at its extension before following prose", () => {
    const path = "/Users/example/Library/Mobile Documents/Quarterly Review/report final.xlsx";
    const prompt = `Read ${path} and compare its totals with the proposal.`;

    expect(parseLocalPathReferences(prompt).map((reference) => reference.normalizedText)).toEqual([
      path,
    ]);
  });

  it("does not truncate a dotted directory component followed by spaces", () => {
    const path = "/Users/example/Archive.json Data/Source Files";

    expect(candidateLocalPathsFromText(`Inspect ${path}`)).toEqual([path]);
  });

  it("still infers a standard user folder when it is requested outside an absolute path", () => {
    const path = "/Users/example/Library/Mobile Documents/report.md";
    const prompt = `Read ${path}, then list my Downloads folder.`;

    expect(candidateLocalPathsFromText(prompt)).toEqual([path, "~/Downloads"]);
  });

  it.each([
    [
      "Inspect /Users/example/Projects/Supplier Review please",
      ["/Users/example/Projects/Supplier Review"],
    ],
    [
      "Inspect /Users/example/Projects/Supplier Review and delete /Users/example/old.txt",
      ["/Users/example/Projects/Supplier Review", "/Users/example/old.txt"],
    ],
    [
      "Compare /Users/example/Projects/Supplier Review and /Users/example/Projects/Vendor Review",
      ["/Users/example/Projects/Supplier Review", "/Users/example/Projects/Vendor Review"],
    ],
    [
      "Read /Users/example/Research Sets/vendor data.parquet please summarize it",
      ["/Users/example/Research Sets/vendor data.parquet"],
    ],
  ])("never widens an unquoted approval target into trailing prose: %s", (prompt, paths) => {
    expect(candidateLocalPathsFromText(prompt)).toEqual(paths);
  });

  it.each([
    "Inspect /Users/example/Projects/Supplier Review for anomalies",
    "Inspect /Users/example/Projects/Supplier Review contents",
    "Inspect /Users/example/Projects/Project Notes because I need context",
    "Inspect /Users/example/Projects/Project Notes with the proposal",
    "Inspect /Users/example/Projects/Project Notes in read-only mode",
  ])("rejects an ambiguous unquoted target instead of enlarging approval scope: %s", (prompt) => {
    expect(candidateLocalPathsFromText(prompt)).toEqual([]);
  });

  it("never manufactures authority from URLs, links, slash commands, examples, or remote file URIs", () => {
    const references = parseLocalPathReferences([
      "Visit https://example.com/docs/start and [the guide](docs/start.md).",
      "Use /travel only as a slash command.",
      "An example path is 'mock_data/example.json'.",
      "Reject file://remote-host/private/data.txt.",
    ].join(" "));

    expect(references).toEqual([]);
  });

  it("retains source spans without Markdown delimiters or terminal prose punctuation", () => {
    const text = "Open `mock_data/a file.json`, then /Users/example/report.txt.";
    const references = parseLocalPathReferences(text);

    expect(references.map((reference) => text.slice(reference.sourceSpan.start, reference.sourceSpan.end))).toEqual([
      "mock_data/a file.json",
      "/Users/example/report.txt",
    ]);
  });
});
