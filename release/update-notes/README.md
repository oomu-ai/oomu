# Versioned update notes

Before preparing a new public release, add `{productVersion}.json` here with
`schemaVersion: 1`, the exact release version, and reviewed notes for all 12
supported locales. The updater-asset command rejects missing, partial, oversized,
or version-mismatched notes. Do not add notes for an already-distributed version;
changed public binaries always receive a higher semantic version.
