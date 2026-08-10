# OOMU 0.1.10 — Public Beta

OOMU 0.1.10 makes Project work dependable from the first file read through the finished result.

## What’s improved

- **Project files open when you name them.** OOMU can read an exact file from the folder attached to the active Project without asking you to attach it again.
- **Natural file references work.** A folder and filename in your request resolve to the approved Project source while remaining inside that Project’s boundary.
- **Multi-file analysis finishes cleanly.** OOMU can read several named Project files, keep their verified results together, and deliver one grounded response.
- **Cloud approval is calm and predictable.** When one cloud reply needs several private tool results, one approval covers that reply instead of interrupting you repeatedly.
- **Auto-route resumes after approval.** Confirming a cloud handoff no longer leaves the reply bound to a route that cannot finish.
- **Daily workflows keep the time you chose.** Requests such as “every morning at 8:00 AM” preserve 8:00 AM and bind every named Project input to the scheduled workflow.
- **Project reports use the real source documents.** PDF report creation can combine spreadsheets and Word documents from the Project folder before producing the verified artifact.
- **New work asks again.** A new reply, destination, model, or expired approval still requires a fresh decision.

## Security and data integrity

- Project reads remain restricted to the active Project’s approved roots. Files outside those roots stay blocked.
- Model-visible Project results use Project-relative paths instead of exposing the Mac’s absolute host paths.
- Every private cloud payload still receives its own signed, verified receipt even when one approval covers the reply.
- An approval cannot cross reply, session, generation, provider, model, representation, or expiry boundaries.
- Scheduled Project work keeps exact file bindings and fails visibly if an approved source is missing or changed.

## Install or upgrade

### Upgrading from OOMU 0.1.9

Choose **Help → Check for Updates…** in OOMU 0.1.9. You can also download `OOMU-0.1.10.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.10.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.10 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
