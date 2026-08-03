# Source module ownership and non-growth policy

OOMU's human-authored source is governed by four mechanical ratchets:

1. `scripts/check-rust-file-lines.sh` enters the parser-backed source-quality
   gate. It enforces exact shrink-only exceptions for physical lines, bytes,
   maximum physical-line bytes, function length, and branch complexity across
   Rust, production TypeScript/TSX, and release/gate scripts.
2. `scripts/check-module-cycles.mjs` builds the renderer file graph and native
   top-level module graph. Existing strongly connected components are recorded
   exactly; a new cycle fails, removing a cycle requires lowering the baseline,
   and forbidden architectural edges fail independently of cycle membership.
3. `scripts/check-unused-exports.mjs` resolves symbols from production entry
   points through aliases, default exports, re-exports, and dynamic imports.
   Test-only consumers are reported separately and never establish production
   reachability.
4. `scripts/check-repository-hygiene.mjs` rejects tracked build/cache output
   anywhere in the repository and rejects duplicate or disallowed assets.

## Native ownership

| Domain | Owning modules | Allowed direction |
| --- | --- | --- |
| Foundation | `foundation::{clock,digest}` | Standard library and reviewed crypto/serialization crates only |
| Persistence | `db`, `persistence_health`, `secret_store` | Foundation, schema/value types, OS storage |
| Security | `security`, `shield_gate`, `network_policy`, `redaction` | Foundation and narrow persistence audit boundaries |
| Inference | `inference`, `gemma`, `native_runtime` | Foundation, model configuration, security boundaries |
| Orchestration | `agentic_loop`, `agent_manager`, `workflow_*`, `taskflow` | Persistence, inference, and security through typed contracts |
| Integrations | `gateway`, `mcp`, `knowledge`, `tools` | Security and persistence; never renderer authority |
| App shell | `lib.rs`, Tauri commands | Typed domain commands; no duplicated business implementation |

New foundation-to-domain edges are forbidden by review. Existing native cycles
are debt baselined by exact membership and cannot grow.

## Renderer ownership

`src/lib` owns pure contracts and algorithms, `src/hooks` owns native
subscriptions, `src/context` owns application state composition, and
`src/app/components` owns presentation. Extracted presentation/domain modules
must not import their former monolithic parent, which prevents compatibility
facades and circular ownership.

Generated Tauri permissions and build output are excluded from human-source
line limits only by their explicit directory classification. Tests remain in
the line ratchet because large test fixtures are also maintenance risk.

## P0 domain reservation and handoff

`scripts/p0-domain-ownership.json` is the mechanical ownership contract for
Sprints 225 through 231. Its roots are reserved, every source file added below
them is capped at 750 lines, and its native and renderer cycle nodes are always
present in the module graph even before a domain has source files. Sprint 232
integrates these domains but does not create another runtime or authority model.

| Sprint | Exclusive domain roots | Shared handoff files |
| --- | --- | --- |
| 225 | `src-tauri/src/projects/`, `src/app/components/projects/` | migrations, `knowledge.rs`, `memory_ledger.rs`, provider policy preflight, AppShell navigation |
| 226 | `src-tauri/src/tasks/`, `src/app/components/tasks/` | migrations, runtime adapters, generated command permissions, startup recovery registration |
| 227 | `src-tauri/src/connectors/`, `src/app/components/integrations/` | MCP adapters, `secret_store.rs`, setup routing, diagnostics adapters |
| 228 | `src-tauri/src/routines/`, `src/app/components/routines/` | workflow scheduler adapters, helper packaging, Shield and gateway adapters |
| 229 | `src-tauri/src/browser_automation/`, `src/app/components/browser_automation/` | `native_browser.rs`, local-context grants, Shield adapters, split-view adapter |
| 230 | `src-tauri/src/artifacts/`, `src/lib/artifacts/`, `src/app/components/artifacts/` | schemas, helper preparation, Task/Project adapters, file export boundary |
| 231 | `src-tauri/src/delegation/`, `src/app/components/delegation/` | `agent_manager.rs`, context/model adapters, task and artifact contribution adapters |
| 232 | No new core domain | Shield/trust adapters, evidence presentation, templates, golden-task harness, release evidence |

The shared P0 contract files are `src/lib/p0Contracts.ts`,
`src-tauri/src/p0_contracts.rs`, and
`schemas/p0-contract-v1-vectors.json`. Their identifiers, states, evidence
classes, and event envelope may only change through an explicit version bump.

`ChatScreen.tsx`, `db.rs`, `lib.rs`, `inference/mod.rs`, and
`workflow_runtime.rs` are registration or delegation seams only for new P0
work. A reviewed sprint may advance a seam baseline only for migration
descriptors, command registration, startup registration, or a typed adapter
call. Domain implementation remains forbidden in these files, and each
accepted ceiling becomes the next non-growth ratchet.
