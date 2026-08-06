# OOMU 0.1.6 — Public Beta

OOMU 0.1.6 is a focused reliability release for the first conversations on a new Mac and for longer DeepSeek chats.

## What’s improved

- **Auto-route starts cleanly after local chat.** When the same local model is already serving a conversation, Auto-route reuses that exact resident model instead of loading a second copy. This prevents the KV-cache decode failure that could interrupt an early task on a clean installation.
- **The fix works with the model you chose.** Reuse is based on the verified model directory, not a hard-coded model name or size. It applies equally to E2B, E4B, 12B, and other compatible local GGUF models. If Auto-route uses a different classifier model, OOMU keeps the models isolated.
- **DeepSeek no longer ends a turn with hidden reasoning and no answer.** If DeepSeek consumes a response on internal reasoning without returning visible text, OOMU retries that turn once with extended reasoning off. Hidden reasoning is never displayed or stored, and the recovery cannot loop.

## Security and data integrity

- OOMU shares only the resident native model handle; the chat and classifier keep independent state so changing a chat model cannot silently change the classifier.
- DeepSeek recovery is limited to DeepSeek reasoning-only responses before any visible answer or tool result is emitted. Other providers and ordinary DeepSeek answers are unchanged.
- Existing permission, approval, receipt, and local-data boundaries remain in force.

## Install or upgrade

### Upgrading from OOMU 0.1.5

Choose **Help → Check for Updates…** in OOMU 0.1.5. You can also download `OOMU-0.1.6.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.6.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.6 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
