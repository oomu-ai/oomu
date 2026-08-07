# OOMU 0.1.7 — Public Beta

OOMU 0.1.7 is a focused reliability release for updates, Auto-route, cloud replies, PDF attachments, and OpenRouter choice.

## What’s improved

- **Updates restart cleanly.** After OOMU installs an update, it now returns through the app’s normal event loop before relaunching. This prevents the shutdown error that could appear immediately after an update.
- **Auto-route is more dependable across Macs.** OOMU uses a smaller on-device context on lower-memory Macs and gives the local classifier enough time to answer on real hardware, while preserving the larger context on more capable systems.
- **Cloud replies recover from an empty answer.** If Gemini or DeepSeek finishes without visible text, OOMU makes one bounded retry with extended reasoning turned off. The recovery cannot loop.
- **PDF attachments work from either gesture.** PDFs selected with the file picker or dragged into Chat now pass through the dedicated PDF parser and remain available for summarizing and other grounded work.
- **OpenRouter offers more current models.** The built-in OpenRouter list now includes DeepSeek V4 Flash 0731, DeepSeek V4 Flash, Tencent Hy3, Xiaomi MiMo-V2.5, Z.ai GLM 5.2, DeepSeek V4 Pro, NVIDIA Nemotron 3 Ultra (free), and MoonshotAI Kimi K3, with OpenRouter-specific context, output, pricing, and reasoning settings.

## Security and data integrity

- The cloud retry applies only when no visible answer or tool result was produced, runs at most once, and never exposes hidden reasoning.
- PDFs continue to use OOMU’s bounded, sandboxed extraction path. The fix removes only an incorrect raster-image header check that ran before PDF parsing.
- Existing model isolation, native permission, approval, receipt, and local-data boundaries remain in force.

## Install or upgrade

### Upgrading from OOMU 0.1.6

Choose **Help → Check for Updates…** in OOMU 0.1.6. You can also download `OOMU-0.1.7.dmg`, open it, and drag OOMU into Applications.

### New installation

Download `OOMU-0.1.7.dmg`, open it, and drag OOMU into Applications.

## Known limitations

- OOMU 0.1.7 is a public beta for Apple Silicon Macs running macOS 14 or later.
- Checking for and installing updates requires access to OOMU’s official GitHub Releases feed.
