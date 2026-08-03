# OOMU

<p align="center">
  <strong>Trust, built in.</strong>
</p>

<p align="center">
  <img src="docs/images/oomu-workspace-hero.svg" alt="A midnight-blue raven shelters an OOMU workflow that moves from a private local document through visible steps, approval, and optional cloud reach." width="1200">
</p>

## Of course this is how agents should work.

**Powerful AI agents. None of the burden.** OOMU runs your everyday work on your own Mac, calls the cloud only when a task truly needs it, and shows you the plan before it acts.

| | |
| --- | --- |
| **Visible workflows** | See how work moves from request to result. |
| **Human review** | Review plans before actions are taken. |
| **Local where possible** | Keep everyday work close to your data and device. |
| **Clear records** | Keep an understandable record of what happened. |

## Documentation

Install OOMU, build workflows, connect services, and understand the system:

- [Read the documentation](https://oomu.ai/docs.html)
- [Installation and first launch](https://oomu.ai/docs-getting-started.html)
- [System design overview](https://oomu.ai/docs-system-design.html)

## Run from source

```sh
npm ci
npm run tauri:dev
```

See [DEVELOPMENT_SETUP.md](DEVELOPMENT_SETUP.md) for prerequisites, first-run testing, connector setup, and validation commands.

## Repository boundaries

- Fresh installations begin empty. OOMU does not automatically install agents, Mods, projects, chats, or sample data.
- Model weights, generated runtimes, compiled applications, and release candidates are not committed.
- OAuth credential bundles, access tokens, signing identities, and notarization credentials remain local.
- OOMU is built, signed, and notarized on an authorized Mac, not in GitHub Actions.
- Mods are user-installed. This repository ships no installed or bundled Mod packages.

## Community license

OOMU is source-available under the [OOMU Community License](LICENSE.md). It is available only for eligible individual, noncommercial use and is **not** an OSI-approved open-source license. Review the license before use, modification, or redistribution.

## Feedback and security

OOMU does not accept external code contributions. Use [GitHub Issues](../../issues) for reproducible defects, product feedback, feature requests, and documentation corrections. See [CONTRIBUTING.md](CONTRIBUTING.md) for the repository policy.

Report suspected vulnerabilities privately through the repository's GitHub Security Advisory page, following [SECURITY.md](SECURITY.md). Do not include credentials, private user data, or an exploitable proof of concept in a public issue.

© 2026 Eldris, Inc.
