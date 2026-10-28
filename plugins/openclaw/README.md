# LexMount Cloud Browser for OpenClaw

Version 2.0 uses a Skill and OpenClaw's native remote MCP client. It does not
download or execute the LexMount browser CLI. The package identity remains
`@tristanisk/lexmount-cloud-browser` and the runtime ID remains
`lexmount-cloud-browser`.

Requires OpenClaw 2026.10.8 or later and its supported Node.js runtime.
OpenClaw handles tool discovery, transport, OAuth and credential storage.

## Default site and migration

The default is now the international site, `browser.lexmount.com`.
Version 1.2.1 used the CLI and defaulted to `browser.lexmount.cn`.
Existing users who want to keep using China should select China before login.
This package never imports, removes or migrates old CLI credentials, sessions
or persistent website contexts. The separate CLI Skill is unchanged.

Follow [connection setup](skills/lexmount-browser/references/connection.md).
The initial operator configuration is necessary for `openclaw mcp login` on
2026.10.8: that command does not resolve a manifest-only MCP server.

Once configured, explicitly switch to China with:

```sh
openclaw config set mcp.servers.lexmount-browser.url https://browser.lexmount.cn/chatgpt-app/mcp
openclaw mcp login lexmount-browser
openclaw mcp doctor lexmount-browser --probe
```

Use `.com` instead of `.cn` to switch back. This changes only the URL and
preserves the operator's existing filters and other MCP settings. Never use
these commands to overwrite an unknown connection; inspect its configuration
first. OpenClaw stores each site's OAuth credentials separately by full URL.
Login can reuse valid credentials previously saved for the selected site.

There is no custom site dropdown in this release. Use OpenClaw configuration
or ask the agent to follow this Skill's explicit site-switch procedure. Shell
access and normal host authorization still apply. A live gateway may need a
supported configuration reload or an operator-approved restart before new
agent turns use the new endpoint.

## Authentication and browser use

Run the native MCP login command and approve the authorization page. The
registered local callback is `http://127.0.0.1:8989/oauth/callback`. Keep port
8989 available. Remote/headless setups need the host's documented callback
handoff; do not paste access tokens or credentials into chat.

The public OAuth client metadata is shipped in `oauth-client.json`. Its URL
must remain publicly readable and the referenced source branch must be retained
while released packages use it. This is public client metadata, not a secret.
The plugin uses the native CLI login path; Control UI's potentially different
callback URLs are not claimed to work with this client's fixed registration.

Use discovered MCP tools for create, navigate, inspect, interact, screenshot
and close. Website login is separate from LexMount service authorization.
When manual interaction is necessary, use the server-provided browser view or
live-view link and wait for the user. MCP App rendering depends on host settings.

An HTTP success or accepted action is not proof of a visible page result.
Inspect the page after actions, and close only task-owned temporary sessions.
The default OAuth identity belongs to the gateway operator; this package does
not add per-user authorization for untrusted users sharing a gateway.

## Packaging and validation

From `plugins/openclaw`:

```sh
npm pack --ignore-scripts
clawhub package validate .
```

Only `openclaw.plugin.json`, the OpenClaw entry point, the MCP Skill, its
connection reference, this README, public OAuth client metadata, and LICENSE
are included. Rust sources and CLI bootstrap scripts are excluded.

Before publication, install the packed artifact in an isolated OpenClaw profile,
check native plugin and Skill discovery, verify operator overrides and disabled
connections, and complete OAuth plus create/navigate/read/screenshot/close
through the actual host. Bind the ClawHub upload to the reviewed source commit.
Publication is subject to ClawHub security review; upload success is not public
availability.
