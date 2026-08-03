# OOMU Release Evidence Contract

`npm run build:prod` is the only supported distributable-build entrypoint. It must run on macOS from a clean Git tree. Direct release-profile Cargo/Tauri builds, direct Tauri bundles, and Linux packaging fail closed. Internal build and bundle phases require an Ed25519 authorization over the exact build ID and source revision, so setting a public environment marker alone cannot bypass the guard.

## Canonical order

The entrypoint resolves the real Apple toolchain, verifies signing inputs and reviewed entitlements, builds and validates the WhatsApp sidecar, runs dependency and product test suites, compiles without bundling, and creates an unsigned app. It then runs the general and database sanitizers against that exact app, validates `Assets.car`, signs nested code, proves the Developer-ID sidecar can complete a live hello handshake and external disposable-account lifecycle suite, writes post-sign sidecar provenance, and signs the outer app.

The final sanitizer also performs a binary-safe byte scan across packaged frontend resources, native executables, and sidecars for removed fixture names, command surfaces, testing paths, and dated fixture text. The pipeline submits and staples the app, creates/signs/submits/staples the DMG, stages both outputs, extracts and exactly compares the signed app/helper entitlements, verifies codesigning/tickets/architecture, generates an atomic Ed25519 exact-tree manifest, and freezes the canonical candidate. The external clean-Mac harness receives a separate read-only, byte-identical DMG copy. After it exits, the canonical exact-tree manifest, DMG SHA-256, app and DMG signatures, and both staple tickets are verified again. Only after those checks are prior executed results wrapped as artifact-bound evidence records.

## Required immutable records

Every record is `oomu.executed-release-evidence`, has `synthetic: false`, identifies its real executable/component/endpoint/input, records a successful executed command, and matches the exact build ID, 40-character source revision, artifact ID, and artifact digest.

| Evidence type | Maximum age | Required result |
| --- | ---: | --- |
| `apple_toolchain` | 24 hours | coherent Xcode developer directory and SDK, exact system paths, Apple-anchored tool signatures, versions/hashes |
| `sidecar_validation` | 24 hours, never later than either nested E2E result | supply-chain validation, Developer-ID hello, standalone and packaged-app disposable-account lifecycles, post-sign provenance |
| `dependency_audit` | 24 hours | executed production dependency audit |
| `automated_tests` | 24 hours | release-integrity, i18n, typecheck, frontend, sidecar contract tests, Cargo check, Cargo tests |
| `release_sanitizer` | 24 hours | executed explicit-policy scan with zero remaining prohibited files |
| `database_sanitizer` | 24 hours | executed app-tree scan and sanitation result, including an explicit zero-DB result |
| `entitlement_snapshot` | 24 hours | entitlements extracted from final signed app/helper and exactly matched to reviewed profiles |
| `artifact_validation` | 24 hours | valid assets, architecture, final codesign, immutable tree |
| `signing` | 24 hours | identity authority, Team ID, hardened runtime, nested/outer/DMG signing |
| `notarization` | 24 hours | accepted app and DMG submission IDs |
| `stapling` | 24 hours | validated app and DMG tickets, including staged copies |
| `manifest_verification` | 24 hours | exact count/path/type/size/SHA-256, source/build/artifact identity, trusted signature |
| `clean_machine_launch` | 7 days | exact-DMG clean install and successful launch with no repository scripts |
| `release_extension_gates` | 24 hours | every lexically discovered packaged-candidate gate passed with measured JSON evidence |
| `source_provenance` | 24 hours | exact full revision, clean-tree proof, and dependency-lock SHA-256 values |

The signed gate expires at the earliest underlying-record expiration, never at a freshly extended time. Evidence files are mode `0444`; the finalized evidence directory is mode `0555`. The candidate tree is also made read-only and its code signature is rechecked. Tracked historical pre-alpha material is never accepted as current release proof.

## Trust root and secrets

Manifest and gate signatures use the reviewed Ed25519 public key pinned in `scripts/release-manifest.mjs` and `src-tauri/src/audit.rs`. CI must provision the matching private key through `OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH` and the public PEM through `OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH`. Key rotation requires a reviewed source change. The private key, Apple passwords/API key, certificate, and integration-account material must never enter evidence or logs.

The Apple input names are documented in `scripts/sign_env.sh.example`, but the canonical entrypoint never sources or parses an ignored environment file. Values must already exist in the explicit shell or CI environment. Preflight requires a real keychain identity whose certificate Team ID matches `APPLE_TEAM_ID`; final `codesign -d` verification is authoritative.

## External secure runners

The canonical pipeline invokes the three repository-owned entry points under `scripts/release-runners/`. Each submits the exact candidate bytes to the Eldris release lab over mutually authenticated TLS 1.3. The client trusts only the provisioned release-lab CA, presents the scoped CI client identity, bounds request/response sizes, and rejects evidence that does not bind the submitted SHA-256. It has no local or synthetic passing mode.

`scripts/run-whatsapp-integration-harness.mjs` invokes the repository WhatsApp entry point twice and validates both reports against `sidecars/whatsapp-sidecar/integration-evidence.schema.json`. The standalone and packaged-app reports bind the exact signed helper/app, real disposable-account behavior, application tree, main executable, Team ID, build/revision, protocol, and expiration. Unknown fields and secret-bearing keys remain prohibited.

Each sidecar launch must generate a fresh 32-byte random lowercase-hex token, pass it only through `OOMU_WHATSAPP_CHANNEL_TOKEN`, request port `0`, parse the `listener.ready` JSON line from stdout, and connect to the reported loopback port with `?token=<token>`. Missing, reused, or unauthenticated channel tokens are a failed run and must never appear in evidence or logs. An outbound reply counts as delivered only after the correlated `sendMessage.result` event for its request ID; queue acceptance is not delivery evidence.

The clean-machine entry point receives:

```text
--artifact <final.dmg>
--build-id <id>
--source-revision <40-hex>
--artifact-id <id>
--artifact-digest <sha256:tree-digest>
--artifact-file-sha256 <dmg-sha256>
--output <json-path>
```

Its JSON must report `status: "passed"`, `synthetic: false`, the exact identities/digests above, a fresh `completed_at`, endpoint and machine identifiers, installed application and launched executable, exit code zero, and true `installed_from_dmg`, `codesign_verified`, and `stapled_ticket_verified` values. It must explicitly report `repository_present: false` and `repository_scripts_on_path: false`. The OS matrix must contain real macOS 14 latest, macOS 15 latest, and current macOS rows, each with passing voice, Vision, PDF build/render, WhatsApp, local inference, and PDF extraction probes. Missing, stale, mismatched, or mutated reports/artifacts fail release certification.

## CI and outputs

OOMU releases are built, signed, and notarized only on an authorized local Mac. The release entrypoint materializes private inputs as mode-`0600` temporary files, confines signing authority to the protected phase, and removes temporary material unconditionally. Ordinary child commands receive a strict environment allowlist. GitHub workflows never receive Apple signing, notarization, OAuth-canary, or release-manifest private credentials.

The signed release gate is external release-certification evidence; it is intentionally not embedded into the app it hashes, which would create a self-referential artifact digest. `audit.rs` defaults to the app-data `release/pre_alpha` location and therefore remains fail-closed on ordinary consumer installations. A release operator or verification harness may set `OOMU_RELEASE_EVIDENCE_DIR` to an extracted immutable `release/evidence/<build-id>` directory. The Rust verifier then checks the pinned gate signature, every referenced record digest and mode, freshness, and artifact binding. The variable selects a location only; it cannot make unsigned or forged evidence pass.

---

### Premises:
- A release claim is valid only when it is bound to the exact signed artifact and source revision.
- Tool, sanitizer, test, signing, integration, and launch results must come from executed external checks.
- The reviewed public trust root and Apple Team ID are non-secret verification anchors.

### Execution Path:
- Run the canonical entrypoint from a clean macOS tree with provisioned secrets and external runners.
- Reject any missing, stale, synthetic, writable, mismatched, or incorrectly signed evidence.
- Retain the read-only candidate, exact-tree manifest, individual records, and signed aggregate gate together.

### Formal Conclusion:
- A candidate is releasable only when `npm run build:prod` completes and emits a signed `release-gate.json` whose referenced immutable records all verify for that candidate.
