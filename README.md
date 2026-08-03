# OOMU

OOMU is a local-first AI workspace for macOS. It combines private on-device models, optional cloud providers, native Apple integrations, projects, workflows, and permission-aware tool execution in one desktop application.

Fresh installations begin empty: no agents, mods, projects, chats, or sample data are installed automatically. People choose their own model, create only the agents they want, and can skip every optional onboarding action.

## Run from source

```sh
npm ci
npm run tauri:dev
```

See [DEVELOPMENT_SETUP.md](DEVELOPMENT_SETUP.md) for prerequisites, first-run testing, connector setup, and validation commands.

## Repository boundaries

- Model weights, generated runtimes, compiled applications, and release candidates are not committed.
- OAuth credential bundles, access tokens, signing identities, and notarization credentials remain local.
- OOMU is built, signed, and notarized on an authorized Mac—not in GitHub Actions.
- Mods are user-installed. This repository ships no installed or bundled mod packages.

## License

OOMU is source-available under the [OOMU Community License](LICENSE.md). It is not an OSI-approved open-source license. Review the license before use or redistribution.
