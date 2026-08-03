# Desktop OAuth build inputs

The reviewed public client identifiers in this directory may be committed. Downloaded credential
JSON, access tokens, and refresh tokens must never be committed.

Google's desktop client uses PKCE and a loopback redirect. Google's token endpoint also requires the
downloaded Desktop client `client_secret` as a protocol field. Local builds read the ignored
`google-desktop-client.json`; build automation may instead provide
`OOMU_GOOGLE_OAUTH_CLIENT_SECRET`. Google documents that an installed app cannot keep this value
confidential, so it is never treated as an authorization boundary. The downloaded JSON itself,
access tokens, and refresh tokens must never be committed.

Slack work-app access uses Slack's public-client PKCE flow with the exact localhost redirect derived
from `slack-redirect-port.txt`. OOMU exchanges and refreshes user tokens directly with Slack and
ships no Slack client secret. The optional real-time messaging tier requests bot scopes through the
Eldris secure connection service; when that reviewed endpoint and certificate pin are absent, only
the messaging tier is unavailable.

Microsoft 365 uses a public desktop client with PKCE and the exact loopback redirect
`http://127.0.0.1:53683/oauth/callback`. Register that URI on the Mobile and desktop application by
editing the Entra application manifest when the portal does not accept an HTTP IP-literal loopback.
Developer builds may read `OOMU_MICROSOFT_OAUTH_CLIENT_ID`; reviewed builds may provide the same
public ID in `microsoft-public-client-id.txt`. No Microsoft client secret belongs in this repository
or binary.
