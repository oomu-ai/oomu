# OOMU 0.1.5 — Public Beta

OOMU 0.1.5 is a focused continuity release. Complex work now recovers cleanly from interruptions, preserves verified results, and asks for missing choices in plain language.

## What’s improved

- **Complex tasks continue without repeating finished work.** When a multi-step task pauses after creating verified files, OOMU can prepare a fresh plan for the remaining Calendar and Mail steps instead of restarting or entering an approval loop.
- **Existing decision packs are protected and reusable.** OOMU reopens the workbook, presentation, PDF, and source ledger; verifies their structure, hashes, inputs, and evidence; and reuses them only when they are the unchanged results of the same request. It never silently overwrites uncertain files.
- **Missing calendar choices are easy to fix.** If a request does not name a calendar, OOMU now asks which calendar to use and clearly confirms that nothing has changed yet.
- **PowerPoint previews accept valid decks.** Presentations may reuse object identifiers on different slides, while animations and citations are still required to reference an object on the correct slide.
- **Mail results remain trustworthy.** Verified Mail receipts now retain the result details needed by later steps and final confirmation.
- **Evidence-bound work reaches the secure planner.** Requests that combine approved local inputs with document creation are routed to the approval-gated execution path instead of ending with a promise to act later.

## Security and data integrity

- Recovery never treats model narration as proof that work happened. OOMU requires native receipts and re-verifies existing files before continuing.
- Calendar changes and Mail drafts remain approval-gated.
- If existing output cannot be tied to the exact request, OOMU stops and asks for a new empty folder instead of overwriting it.

## Install or upgrade

### Upgrading from OOMU 0.1.4

After 0.1.5 is published, choose **Help → Check for Updates…** in OOMU 0.1.4. You can also download `OOMU-0.1.5.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.5.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.5 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
