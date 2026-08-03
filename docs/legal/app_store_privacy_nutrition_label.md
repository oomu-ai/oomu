# OOMU App Store Privacy Nutrition Label

Last updated: July 10, 2026

## Scope

This document describes OOMU desktop network behavior relevant to App Store privacy review. OOMU does not contact an OOMU or Eldris service when the application starts or when a user accepts or declines the license.

## License Acceptance

OOMU Community License 1.2 is shown in full before licensed functionality becomes available. The application stores only the accepted license version and acceptance timestamp in its local settings file. License review, acceptance, and decline are local operations and do not make a network request.

## App Store Data Collection Position

License review, acceptance, decline, and launch are local operations. They make no OOMU or Eldris network request. OOMU has no operated analytics, advertising identifier, tracking SDK, or telemetry upload, so it does not collect data through an operated service for startup or licensing.

Provider and integration traffic initiated by the user is distinct from collection by OOMU. Users should review the terms and privacy practices of each provider they configure.

## Other Network Behavior

- User-requested or user-enabled DuckDuckGo Lite web grounding sends the search query directly from the user's Mac to DuckDuckGo.
- A configured cloud model provider receives the content necessary to fulfill the user's request when the user selects that provider.
- A configured remote MCP server receives requests only when the user enables and uses that integration.
- Native browser navigation contacts the destination selected by the user or required by the user's task.
- Channel integrations contact the provider selected and configured by the user.

OOMU does not attach a license identifier, startup identifier, compliance identifier, or hidden analytics payload to those requests.

## User Controls

- Automated web grounding is off by default and can be changed in Privacy settings.
- Per-session grounding can be controlled in chat.
- Provider, MCP, browser, and channel activity remains subject to the applicable user action, configuration, and approval boundary.

## Required Review Before Release

Qualified legal/privacy review must confirm the final App Store declarations against the release binary, configured entitlements, provider behavior, and current App Store terminology.

## Implementation References

- `LICENSE.md`: License 1.2 terms and local acceptance condition.
- `src-tauri/src/settings.rs`: Local-only license acceptance state.
- `src/app/components/HomeChrome.tsx`: Blocking, keyboard-accessible complete-license gate.
- `src/app/components/settings/PrivacyPanel.tsx`: User-controlled automated web-grounding setting.
