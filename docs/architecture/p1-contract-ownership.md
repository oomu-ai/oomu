# P1 contract ownership and integration policy

Sprint 233 reserved one implementation lane for each P1 capability while preserving the
Project, Task, evidence, approval, artifact, and authority boundaries established by P0.
The machine-readable source of truth is `scripts/p1-domain-ownership.json`; this document
explains how feature work must consume it.

## Reserved ownership

| Domain | Owning sprint | Native root | Renderer or shared root | Contract ownership |
| --- | ---: | --- | --- | --- |
| Connector expansion | 234 | `src-tauri/src/connectors/microsoft365/` | `src/app/components/integrations/microsoft365/` | Reuses the P0 connector account, Project binding, and approval contracts |
| Workbooks | 235 | `src-tauri/src/artifacts/workbooks/` | `src/lib/artifacts/workbooks/`, `src/app/components/artifacts/workbooks/` | `ArtifactWorkbook` |
| Presentations | 236 | `src-tauri/src/artifacts/presentations/` | `src/lib/artifacts/presentations/`, `src/app/components/artifacts/presentations/` | `ArtifactPresentation` |
| Computer use | 237 | `src-tauri/src/computer_use/` | `src/app/components/computer_use/` | `DesktopObservation`, `DesktopAction` |
| Media | 238 | `src-tauri/src/media/` | `src/app/components/media/` | `MediaAsset` |
| Remote dispatch | 239 | `src-tauri/src/remote_access/` | `src/app/components/settings/` | `RemoteDevice` |
| Capability bundles | 240 | `src-tauri/src/capability_bundles/` | `src/app/components/capability_bundles/` | `CapabilityBundle` |
| Learning | 241 | `src-tauri/src/learning/` | `src/app/components/learning/` | `LearningCandidate` |
| Work graphs | 242 | `src-tauri/src/analysis/` | `src/app/components/analysis/`, `src/app/components/delegation/` | `WorkGraph` |

## Shared seams

Shared command routing, capability manifests, navigation, database migrations, and Task
event projection remain registration seams. Feature logic must stay in its owning root.
Changes to a shared seam are limited to typed command registration, migration descriptors,
navigation registration, and event projection adapters.

All new envelopes use the P1 contract version and the existing P0 identifiers and evidence
vocabulary. A child run can propose work but cannot grant approval or commit a parent-owned
mutation. Signed remote and package envelopes remain untrusted input until their later
domain sprints implement verification and policy evaluation.

## Mechanical gates

The P1 contract gate checks reserved roots, per-file size ceilings, shared fixtures, hero
workflow postconditions, and the presence of schema, command, migration, cycle, and
repository-hygiene ratchets. The global module-cycle checker imports both P0 and P1 domain
manifests so new files cannot introduce a cycle through a previously empty reserved root.

Sprints 234 through 237 now implement connector expansion, workbooks, presentations, and
guarded Mac app control. Their local contract and component gates are executable, while
external account, document-engine, and qualified application/version acceptance remain
explicitly not run. The remaining five domains stay reservation-only until implementation
begins. The current protected P0/shared contract surface is digest-bound directly; files
inside implemented P1 roots cannot silently rewrite that boundary.
