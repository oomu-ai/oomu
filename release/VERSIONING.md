# OOMU public versioning

`release/version.json` is the authority for OOMU's public version, release
channel, public label, intended Git tag, and signed-candidate build number.
Active manifests must agree with that record. `npm run check:version` enforces
the agreement before quality and release-integrity checks can pass.

## Version contract

- Bug fixes increment the patch version: `0.1.0` to `0.1.1`.
- Feature releases increment the minor version: `0.1.x` to `0.2.0`.
- Nightlies target the next feature version and use
  `0.2.0-nightly.YYYYMMDD.N`.
- Release candidates use `0.2.0-rc.N`.
- `1.0.0` is reserved for OOMU's first stable compatibility promise.
- Every signed candidate increments `buildNumber`. A build number is never
  reused, even when a candidate fails qualification or is not published.

The release pipeline binds both the product version and the separate build
number into immutable internal evidence. The public DMG name contains only the
product version so users receive a calm, conventional installer name.

## One-time transition from internal builds

The former `1.257.x` number was an internal placeholder. Version comparison
treats it as newer than `0.1.0`, so an installed internal build cannot
automatically update to the first public beta.

For this transition only, quit OOMU and manually replace the installed
application with OOMU 0.1. Existing user data stays in place because the bundle
identifier and user-data locations do not change. Future public updates resume
from the `0.x.y` contract above.
