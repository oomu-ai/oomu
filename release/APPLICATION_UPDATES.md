# Application update release contract

OOMU checks the public release feed at
`https://github.com/oomu-ai/oomu/releases/latest/download/latest.json`.
Public beta releases must therefore be published, non-draft, and non-prerelease.

Compilation, Apple signing, notarization, stapling, updater packaging, and
updater signing happen only on the trusted local release Mac. GitHub never
receives application signing credentials and never performs those operations.

After the final signed, notarized, stapled, and qualified `OOMU.app` is ready,
run `npm run release:updater-assets` with:

- `OOMU_QUALIFIED_APP_PATH`: the exact final `OOMU.app`.
- `OOMU_UPDATER_OUTPUT_DIR`: a new output directory.
- `OOMU_UPDATE_PUBLISHED_AT`: the intended ISO-8601 release timestamp.
- `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PATH`: the dedicated updater key.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password.
- `OOMU_UPDATER_PUBLIC_KEY`: the matching public key injected when the release app was compiled.

The command refuses a dirty source tree, a non-qualified app, an existing output
directory, mismatched version/locales, or an archive that does not reproduce the
qualified app tree. It writes the archive, signature, `latest.json`, localized
notes, checksums, and a receipt. Uploading these already-built assets to a draft
GitHub Release is a separate publication action. `npm run release:publish-update`
accepts the qualified DMG and completed updater directory, uploads them to a
draft, reads every asset back, compares sizes and SHA-256 digests, and only then
publishes the release. It requires an exact `publish-v{version}` confirmation.
The command uploads bytes only; GitHub never compiles, signs, or notarizes OOMU.

A release is not update-qualified until a clean supported Mac completes a real
signed `N → N+1` installation and independently verifies the installed version,
Apple signature, staple, and preserved user state.
