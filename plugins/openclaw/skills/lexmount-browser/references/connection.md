# Connection and site selection

OpenClaw 2026.10.8 or later is required. This plugin contributes the
`lexmount-browser` MCP server. The default site is international (.com).
The previous 1.2.1 CLI plugin defaulted to China (.cn). Tell upgrading users
about this change before first authorization. CLI credentials are neither
imported nor deleted.

## Initial setup

First inspect `openclaw mcp list` and the effective plugin configuration.
The 2026.10.8 `mcp login` command reads operator-configured servers, so save
an explicit server definition before login when only the plugin default exists.
Do not replace an existing operator override or enable a disabled server without
the user's request. Retain unrelated headers, filters, timeouts and settings.

For a new default connection:

```sh
openclaw mcp set lexmount-browser '{"enabled":true,"transport":"streamable-http","url":"https://browser.lexmount.com/chatgpt-app/mcp","auth":"oauth","oauth":{"scope":"browser.session:create","redirectUrl":"http://127.0.0.1:8989/oauth/callback","clientMetadataUrl":"https://raw.githubusercontent.com/TristanIsK/browser-cli-rs/codex/openclaw-skill-mcp/plugins/openclaw/oauth-client.json"},"connectionTimeoutMs":30000,"requestTimeoutMs":60000}'
openclaw mcp login lexmount-browser
openclaw mcp doctor lexmount-browser --probe
```

Let the user approve the printed authorization URL. OpenClaw handles the
loopback callback. Do not promise a successful connection until the host has
saved authorization and discovered the server's tools. If a gateway is already
running, use its supported reload/restart procedure and start a new agent turn.
Do not restart a shared gateway without operator authorization.

## Explicit site switch

Switch only when the user names the target site. Never change sites to work
around a failed request or an authorization error. The supported endpoints are:

| Site | MCP URL |
| --- | --- |
| International (default) | `https://browser.lexmount.com/chatgpt-app/mcp` |
| China | `https://browser.lexmount.cn/chatgpt-app/mcp` |

For an already configured connection, change only its URL:

```sh
openclaw config set mcp.servers.lexmount-browser.url https://browser.lexmount.cn/chatgpt-app/mcp
openclaw mcp login lexmount-browser
openclaw mcp doctor lexmount-browser --probe
```

To return to international, replace `.cn` with `.com` in the config command.
If no operator definition exists, use the initial setup definition with the
requested URL instead. Read the host's help before executing commands on an
unsupported host version.

OAuth identity is scoped by server name and full server URL. Do not copy,
rewrite or reuse tokens between sites. A previously authorized target may not
need another approval; expired or missing authorization must be renewed there.
Changing the endpoint does not move browser sessions, contexts or website
logins. Keep existing sessions on their original connection and do not send
old session/context IDs to the new site. Finish or close temporary sessions
before switching when authorized; never interrupt an active user takeover.

If an agent cannot run host configuration commands under current permissions,
show the operator the commands. Do not bypass tool policy or change approvals.
This version uses shared operator OAuth; do not represent it as per-user login
isolation for untrusted users of a shared gateway.
