# OOMU 0.1.8 — Public Beta

OOMU 0.1.8 makes on-device models calmer, easier to manage, and more dependable across sessions.

## What’s improved

- **OOMU rests when you do.** Background workers and schedulers now sleep when there is no work, ending the sustained CPU use seen while OOMU was idle. They wake automatically when new work arrives.
- **Local model folders are easier to repair.** If more than one GGUF model is placed in the same folder, Settings explains the problem in plain language and shows exactly how to organize the files.
- **More GGUF models keep a stable identity.** OOMU reads model metadata instead of relying on a narrow filename pattern, so compatible models remain recognizable after a file or folder is renamed.
- **Startup model changes are reliable.** Choosing a different on-device startup model now updates the active route cleanly and preserves the previous working model if preparation fails.
- **Auto-route keeps the model you chose.** Session binding and per-turn recovery now agree about which on-device model should classify and answer the request.
- **Settings use clearer language.** Privacy, device identity, Office export, model setup, and agent controls now explain what OOMU is doing without internal terminology.

## Security and data integrity

- Idle-work changes do not weaken background-task verification or execution receipts.
- Model identity is derived from bounded GGUF metadata and file evidence; model content is not uploaded.
- Startup-model changes are transactional: OOMU keeps the last verified model ready when a new selection cannot be prepared.
- Existing native permission, approval, receipt, and local-data boundaries remain in force.

## Install or upgrade

### Upgrading from OOMU 0.1.7

Choose **Help → Check for Updates…** in OOMU 0.1.7. You can also download `OOMU-0.1.8.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.8.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.8 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
