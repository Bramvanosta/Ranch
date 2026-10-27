# Security policy

Ranch holds credentials to Redis servers and SSH bastions, can delete jobs, and Ranch server exposes all of that over HTTP. Security reports are taken seriously.

## Reporting a vulnerability

**Please do not open a public issue.** Report it privately through GitHub: [Security › Report a vulnerability](https://github.com/Bramvanosta/ranch/security/advisories/new).

Include what is affected (desktop app, Ranch server, `ranch-mcp`), the version, how to reproduce it, and what an attacker could do with it. You will get an answer within a week. Once a fix is released, the advisory is published with credit to you, unless you prefer otherwise.

## Supported versions

Fixes go into the latest release. The desktop app updates itself; Ranch server is updated by pulling the newest image.

## Scope

In scope, for example:

- reading or changing a stored secret (Redis or SSH password, key path) without the user's action;
- an agent going around the approval of writes, or the read-only setting of a connection;
- writing to a locked production connection without unlocking it;
- getting past Ranch server's authentication, or making a signed-in browser act without the user (CSRF);
- an update installed without a valid signature.

Out of scope: attacks that need full control of the user's machine or of the Redis server itself, and Ranch server exposed without TLS or with `RANCH_ALLOW_NO_AUTH=1`, which the README warns against.
