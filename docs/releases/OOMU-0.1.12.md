# OOMU 0.1.12 — Public Beta

OOMU 0.1.12 makes Auto-route dependable for demanding Project work while keeping private data under your control.

## What’s improved

- **Complex Project work reaches the capable model.** Multi-file evaluations, cross-source reconciliation, and multi-scenario trade-off analysis now route to the configured cloud model instead of being forced through the on-device model.
- **Approval continues the same request.** Project cloud approval and private-file approval now carry the same saved turn through to completion. OOMU will not ask you to repeat an approval you already gave for that turn.
- **Private files remain private until approval.** OOMU names the files it needs to send and waits. Choosing **Keep on this Mac** does not send them.
- **Simple work stays on device.** Bounded requests such as short rewrites continue to use the selected local model.
- **Auto-route is ready after launch.** Startup now verifies the classifier lane before reporting that Auto-route is available.

## Security and data integrity

- Semantic routing evaluates the user’s objective without mixing private file contents or internal attachment receipts into the classification prompt.
- Cloud consent is bound to the accepted turn, Project, provider, model, and approved file set.
- An approved turn can pass each required privacy gate without losing its verified identity, then dispatches once; stale, expired, or mismatched continuations remain rejected.
- Existing Project folder authorization, native file receipts, and local execution boundaries remain enforced.

## Install or upgrade

### Upgrading from OOMU 0.1.11

Choose **Help → Check for Updates…** in OOMU 0.1.11. You can also download `OOMU-0.1.12.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.12.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.12 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
