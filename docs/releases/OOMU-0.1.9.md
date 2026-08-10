# OOMU 0.1.9 — Public Beta

OOMU 0.1.9 makes scheduled email checks dependable from review through the first verified run.

## What’s improved

- **Reviewed schedules stay intact.** Confirming a scheduled email check preserves the workflow the user reviewed instead of replacing it with an incomplete action.
- **Test runs start promptly.** A requested one-time verification run executes immediately while the recurring schedule remains available for its next interval.
- **Failures remain visible.** If a scheduled check cannot finish, OOMU keeps the failure on screen instead of silently ending the task.
- **Idle scheduling remains efficient.** The scheduler wakes when new work arrives without continuously consuming CPU while nothing is due.

## Security and data integrity

- Mail reads continue to use OOMU’s native permission, scope, and receipt boundaries.
- Creating a schedule does not grant broader access to Mail or allow the model to bypass native authorization.
- A failed test run does not erase or falsely complete the reviewed recurring schedule.

## Install or upgrade

### Upgrading from OOMU 0.1.8

Choose **Help → Check for Updates…** in OOMU 0.1.8. You can also download `OOMU-0.1.9.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.9.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.9 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
