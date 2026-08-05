# OOMU 0.1.4 — Public Beta

OOMU 0.1.4 is a focused reliability release. Connected tools answer more consistently, and recovery can no longer prevent you from using the rest of OOMU.

## What’s improved

- **Tool results arrive on the first request.** When a connected tool needs approval or completes a lookup, OOMU now preserves the verified result until the answer is ready instead of leaving a blank response or asking you to repeat yourself.
- **Native continuations stay trustworthy.** One-use execution receipts are consumed only when the model is ready to produce the response, so an approval step cannot accidentally invalidate a legitimate Mail, document, or other native-tool result.
- **Recovery is no longer a blocker.** **Check and recover** can repair compatible early-beta storage after verifying it, while **Continue** always lets you use unaffected parts of OOMU if recovery cannot finish immediately.

## Security and data integrity

- OOMU accepts the affected early-beta database state only after verifying its required tables, indexes, columns, database integrity, and foreign-key integrity.
- Unrecognized migration changes and incomplete schemas still fail closed.
- Temporary work is reconciled into durable storage only after verification, and remains recoverable when conflicts require your decision.

## Install or upgrade

### Upgrading from OOMU 0.1.3

After 0.1.4 is published, choose **Help → Check for Updates…** in OOMU 0.1.3. You can also download `OOMU-0.1.4.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.4.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.4 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
