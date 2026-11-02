# Security policy

RiffBoard is built to run on the public internet, so security reports are very welcome.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting:
**[Report a vulnerability](https://github.com/Imdabossou/RiffBoard/security/advisories/new)**.

Don't open a public issue for security problems. Include what you found, how to reproduce it, and what an attacker could do with it.

## Supported versions

Only the latest release on `main` gets security fixes.

## Scope

Particularly interesting:

- Any way for a visitor to change server state beyond starting a run and posting a score.
- Any way to read files outside the song library, or to reach the `/api/admin/` routes from outside the server.
- Ways to post scores that the server's checks should have rejected.
- Cross-site scripting through song metadata, names or scores.

Out of scope: forging a plausible score by replaying a real-time run with fake hit counts. Note detection runs in the browser, so the server can only make cheating tedious, not impossible (see the README).
