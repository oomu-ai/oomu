# OOMU updater signing key runbook

The updater key is independent of Apple Developer ID signing and independent of
OOMU’s release-evidence key. Its public half is embedded in each release build;
its private half never enters the repository, application bundle, GitHub, logs,
or release assets.

## Provisioning

1. On the trusted local release Mac, run `tauri signer generate` and save the
   encrypted private key in the restricted local release-credential directory.
2. Set the private key file and password only through the local release
   environment. Set `OOMU_UPDATER_PUBLIC_KEY` to the generated public value for
   the release compile and updater-asset generation.
3. Keep one encrypted offline backup under the same two-person recovery policy
   as the Apple release credentials. Verify the backup by signing and rejecting
   a disposable test payload before the first public update.
4. Record the public-key digest in release evidence. Never record the private key
   or password.

## Rotation

An installed OOMU version can trust only the public key embedded when it was
built. Normal rotation therefore uses a bridge release signed by the old updater
key and containing the new public key. Only after that bridge is broadly
installed may later updates be signed solely by the new key. A new key must pass
the complete signed `N → N+1` qualification before publication.

## Suspected compromise

Stop publication immediately. Remove the affected release from the public feed,
preserve evidence, and publish no unsigned or browser-download workaround. If a
safe old key remains available, ship a higher bridge version that embeds a new
key. If no trusted updater path remains, distribute a freshly Apple-signed and
notarized full installer through the official release page and explain the
manual recovery plainly.
