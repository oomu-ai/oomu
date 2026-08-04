# OOMU 0.1.3 — Public Beta

OOMU 0.1.3 makes everyday work more dependable and establishes a secure path for future updates.

## What’s new

- **More natural file creation.** Requests for PDF, Word, PowerPoint, and Excel files now map reliably to the intended format. If you do not name a destination, OOMU creates a useful filename in Downloads; requests for Desktop or Documents are honored.
- **Links open correctly.** Links in chat messages, source citations, and license content now open in your default browser.
- **More reliable agent saving.** Saving a new agent now keeps one identity across retries and recognizes the final saved model name correctly.
- **Better Contacts recovery.** OOMU can now complete the native macOS Contacts permission flow and guide you through recovery when access has not been granted.

## Security & privacy

- **Clear approval for public search.** Choose **Just Once** or **For This Chat**. Chat approval is bound to the current chat, agent, search service, and tool; it is cleared when the chat is removed or OOMU closes.
- **Only the approved query is sent.** OOMU sends the approved search terms—not the full conversation—to its public search provider.
- **Safer external links.** OOMU opens only standard HTTP or HTTPS links and rejects links containing embedded credentials.

## Updates & release integrity

0.1.3 is the first OOMU release with built-in updates.

- Choose **Help → Check for Updates…** at any time. OOMU also checks automatically at most once a day.
- The update window shows what is new, download progress, verification status, and clear choices to install, restart later, be reminded, or skip that version.
- OOMU accepts updates only from the official published GitHub release feed and verifies the update signature before installation. If verification fails, the update is not installed.
- The release process now binds the app, installer, update package, and release tag to the same source revision, verifies signatures and checksums, and reads uploaded assets back before publication.

## Install or upgrade

### Upgrading from OOMU 0.1.2

OOMU 0.1.2 does not include the built-in updater, so this upgrade requires one manual installation:

1. Quit OOMU.
2. Download `OOMU-0.1.3.dmg` from the assets on this release.
3. Open the disk image and drag OOMU into Applications. Choose **Replace** if macOS asks.
4. Open OOMU 0.1.3.

After 0.1.3 is installed, future published releases can be installed from **Help → Check for Updates…**.

### New installation

Download `OOMU-0.1.3.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.3 is a public beta for Apple Silicon Macs running macOS 14 or later.
- OOMU 0.1.2 cannot update itself to 0.1.3; use the manual upgrade steps above.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
