const testingRoot = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data";
const outputDirectory = `${testingRoot}/ship_test_01`;

export const scenarioOneDecisionPackPrompt = [
  "prepare a board-ready supplier decision pack.",
  "Read /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mocked_data/supplier_proposals.json and q3_strategic_vendor_proposals.txt from my testing folder.",
  "Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation.",
  "Cite every web claim with its URL and access time.",
  "Create a new ship_test_01 folder in the testing folder and deliver four real files: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md.",
  "The workbook must contain source data, formulas, exception flags, and a recommendation sheet.",
  "The presentation and PDF must be executive-ready and mutually consistent.",
  "Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to recipient@example.com summarizing the recommendation and listing the four output files.",
  "Do not send the email.",
  "Ask for any required approvals and continue from the exact stopped step after I approve.",
  "Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.",
].join(" ");

export const scenarioOneDecisionPackSteps = [
  {
    step: "Build and verify the complete supplier decision pack from the approved evidence.",
    tool: {
      kind: "registered_task_tool",
      operation: "create_decision_pack",
      arguments: {
        title: "Supplier Decision Pack",
        locale: "en-US",
        inputPaths: [
          `${testingRoot}/mock_data/supplier_proposals.json`,
          `${testingRoot}/mock_data/q3_strategic_vendor_proposals.txt`,
        ],
        researchQueries: ["official current fuel conditions", "official current freight conditions"],
        analysisInstructions: "Reconcile every quoted amount and margin and identify every exception.",
        outputDirectory,
        outputs: {
          workbook: "supplier_decision.xlsx",
          presentation: "supplier_decision.pptx",
          pdf: "supplier_decision.pdf",
          sources: "sources.md",
        },
      },
    },
    risk_level: "high" as const,
  },
  {
    step: "Find the earliest conflict-free time and create the tentative review event.",
    tool: {
      kind: "registered_task_tool",
      operation: "create_conflict_free_calendar_event",
      arguments: {
        calendarName: "OOMU Test",
        title: "Supplier Decision Review",
        day: "next_weekday",
        windowStartLocal: "13:00",
        windowEndLocal: "16:00",
        durationMinutes: 30,
        location: "",
        notes: "Review the verified supplier decision pack.",
        availability: "tentative",
      },
    },
    risk_level: "high" as const,
  },
  {
    step: "Save the receipt-bound decision summary as an unsent Mail draft.",
    tool: {
      kind: "registered_task_tool",
      operation: "draft_decision_pack_email",
      arguments: {
        to: "recipient@example.com",
        subject: "Supplier Decision Review",
        expectedOutputPaths: [
          `${outputDirectory}/supplier_decision.xlsx`,
          `${outputDirectory}/supplier_decision.pptx`,
          `${outputDirectory}/supplier_decision.pdf`,
          `${outputDirectory}/sources.md`,
        ],
      },
    },
    risk_level: "high" as const,
  },
];
