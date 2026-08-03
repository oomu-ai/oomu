# Novice-first UI standard

Every OOMU surface must be understandable without product training or knowledge of the
runtime architecture. This standard applies to new work and to an existing surface when
that surface is materially changed.

## Default-view contract

The default view answers exactly three questions:

1. **What is this?** Use one familiar title and one short purpose sentence.
2. **Is it ready?** Translate state into a plain, actionable sentence. Never print a raw
   enum, diagnostic, provider response, or implementation identity.
3. **What should I do next?** Present one clear primary action. Secondary actions must not
   compete visually with it.

The user should not need explanatory onboarding to discover these answers.

## Progressive disclosure

Show the result or preview before implementation evidence. Put scopes, routes, source
details, checks, version history, and advanced export choices behind a plain **Details**
disclosure or a just-in-time confirmation. Reveal a consequential control only where its
target and effect are clear.

Consent is always just in time. Before opening an authorization page, show the exact
operation, scopes, and destinations, then require an explicit Continue action. Never ask
for unrelated capabilities in the same consent step.

## Language boundary

All renderer copy uses `t()`. Stable enums and codes cross an enum-to-label boundary with
a localized unknown fallback. URLs, account names, user content, file contents, and OAuth
scope identifiers may remain literal data. Backend error prose, health detail, repair prose,
raw JSON, builder/renderer identities, and engine/version strings never reach the glass.

Use familiar nouns and verbs. The user-facing library is **Documents**. File choices are
**Word document and PDF**, **Excel spreadsheet**, and **PowerPoint presentation**. Internal
Artifact, IR, and package names remain internal.

### One noun per concept

Use one user-facing noun for each concept everywhere it appears. A rail label, tab label,
screen heading, empty-state noun, and action label must agree; implementation aliases and
legacy route IDs stay behind the language boundary. If a section is called **Scheduled**,
its empty state and actions also say **Scheduled**, not Routines. If the library is called
**Documents**, user-facing actions say Documents, not Artifacts.

## Documents contract

Documents are created from the work that needs them. Project and Task ownership come from
the current Task, chat, or routine context; the library never requires users to reverse-pick
both before creation. Word/PDF, spreadsheets, and presentations normalize into one shared,
preview-first review shell. Verification and revisions use plain language, while technical
evidence stays under Details.

## Review requirements

- Tests must prove purpose, readiness, and one primary next action are visible by default.
- Tests must prove technical labels and evidence are hidden until Details or a consent
  confirmation opens.
- Tests must prove unavailable state never implies success, connection, verification, or
  recalculation.
- The executable novice-first gate scans governed copy and surfaces for banned glass leaks,
  contextual ownership selectors, raw backend prose, and divergent review shells.
