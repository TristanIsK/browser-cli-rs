# Lexmount Browser CLI (Rust)

Native Rust SDK and command-line client for Lexmount cloud browsers. The binary is
named `browser-cli` so existing agent instructions can migrate without changing
their command prefix.

## Build

```bash
cargo build --release
./target/release/browser-cli version
```

Configuration is loaded from `LEXMOUNT_API_KEY`, `LEXMOUNT_PROJECT_ID`, optional
`LEXMOUNT_BASE_URL` (default `https://api.lexmount.cn`), and optional
`LEXMOUNT_REGION`. `browser-cli auth login` uses a loopback callback and PKCE;
pass `--client-name "<name>"` to identify the calling Agent in the approval page,
or omit it to use `Agent`. Credentials are stored at
`~/.config/lexmount/browser-cli/credentials.json` with mode `0600` on Unix and
are never printed.

All commands emit one JSON document. Run `browser-cli --help` for the complete
surface.

### Errors and output pipes

Successful commands write `{"ok":true,"data":...}` to stdout. Runtime failures
write `{"ok":false,"error":"...","message":"..."}` to stderr and exit with
status 1. JavaScript evaluation failures keep the `cdp_error` category, but now
include the browser's error summary and, when provided, one-based line/column
positions. For example, a missing selector reports `Error: selector not found`
instead of only `Uncaught`. This also applies to actions implemented with
evaluation, such as `click` and `fill`; it does not retry or fix the action.

The summary is the first line of the exception description (up to 1024 Unicode
characters plus a truncation marker), falling back to a primitive thrown value
or CDP's error text when needed. The CLI does not append the stack trace,
source URL, evaluated expression or remote object preview. Exception messages
are page-provided text and may themselves contain sensitive data; inspect them
before sharing logs.

If a stdout consumer closes its pipe early (for example, `... | head`), an
otherwise successful command exits normally without a BrokenPipe panic.
Other output write/flush errors still exit with status 1. An operation that
failed still exits with status 1 even if stderr is closed and cannot report the
error. These source changes require a new binary; updating Skill instructions
does not change an already installed CLI.

## Cloud runtime proxies

Version 1.2.1 routes CDP WebSocket connections through the environment's HTTP
CONNECT proxy. `wss://` uses `HTTPS_PROXY` and `ws://` uses `HTTP_PROXY`, with
`ALL_PROXY` as the fallback; lowercase variables and `NO_PROXY` are handled by
the same proxy matcher used by the HTTP client. Target DNS is resolved by the
proxy. Proxy Basic authentication stays on CONNECT and is not forwarded to CDP.
TLS certificate and hostname checks remain enabled. A rejected proxy request
never falls back to a direct connection.

This transport currently accepts `http://` proxies only; HTTPS-to-proxy and
SOCKS proxies return an explicit unsupported configuration error. Direct and
proxied connections share one 15-second network connection budget, covering
DNS, TCP, CONNECT and TLS/WebSocket handshake across all redirects. CONNECT
headers are limited to 16 KiB. Each blocking network operation uses the remaining
budget; a slow peer cannot restart it by sending another byte.

OS DNS resolution preserves hosts/VPN configuration. Two process-wide workers
and four queue slots bound background work. The caller stops waiting at its
deadline; an in-flight OS lookup cannot be cancelled, and its late result cannot
open a connection. Expired queued lookups are skipped. If the pool is saturated,
new hostname lookups fail with `DNS resolver busy; retry later` until workers
recover. Numeric addresses bypass DNS.

Connection timeouts exit the CLI with status 1 and a JSON error on stderr, e.g.
`{"ok":false,"error":"timeout","message":"request timed out: CDP connection (stage: proxy_dns, budget: 15s)"}`.
Stage names distinguish `proxy_dns`/`target_dns`, `proxy_tcp`/`target_tcp`,
`proxy_connect`, and `websocket_handshake`/`tls_websocket_handshake` without exposing
URLs or credentials. A connection timeout occurs before any CDP command is sent.
The budget ends at the WebSocket upgrade: REST session requests, CDP target
attachment, and later browser actions retain their existing timeout behavior.
It is not a deadline for an entire CLI command or Agent turn, nor does it trigger
automatic action retries. These changes require a new CLI release; published
1.1.15 and 1.2.0 binaries do not acquire them by updating Skill instructions.

## Select a page in a multi-tab session

Explicit page selection is introduced in version 1.2.0. Check that the installed
binary's `browser-cli action --help` lists `--target-id`; the published 1.1.15
binary does not have it. The package version and both bootstrap scripts target
1.2.3 together. Merging or building this source does not publish release assets:
bootstrap can install 1.2.3 only after its binaries and checksums are published
to COS. Until then, use a source build for local verification.

Every `action` command accepts an optional `--target-id`. Obtain the page's CDP
target ID from `session targets` (the page entry's `id` in a DevTools `/json`
listing), then pass it on **each** action that should use that tab:

```bash
browser-cli session targets --session-id SESSION_ID
browser-cli action snapshot --session-id SESSION_ID --target-id PAGE_ID
browser-cli action fill --session-id SESSION_ID --target-id PAGE_ID --selector '#query' --value 'search terms'
browser-cli action click --session-id SESSION_ID --target-id PAGE_ID --selector '#search'
# If this opened a new tab, list targets again and select the result page.
browser-cli session targets --session-id SESSION_ID
browser-cli action wait-selector --session-id SESSION_ID --target-id RESULT_PAGE_ID --selector '#results'
browser-cli action snapshot --session-id SESSION_ID --target-id RESULT_PAGE_ID
```

The option can also precede the action subcommand:
`browser-cli action --target-id PAGE_ID snapshot --session-id SESSION_ID`.
It applies to all actions, including `open-url`, `screenshot`, `pdf`, and `raw`;
their JSON result shapes are unchanged. It is not a session/context option.

The browser session ID and page target ID identify different things. An explicit
target must be an existing page in that browser session. A missing/closed target
returns `not_found`; a non-page target returns `configuration_error`. If the page
closes between discovery and attachment, the CDP error is propagated. None of
these cases falls back to another tab or creates a blank page.

Without `--target-id`, the existing default is unchanged: select the first page
returned by CDP, or create `about:blank` if no page exists. That default is **not**
a guarantee to follow a popup or select the most recently used tab. Explicit
selection is per invocation; there is no persisted active-page state or automatic
new-tab switching. Select by the task's expected URL/title, not list position,
and inspect again when there are multiple plausible pages.

SDK callers can use `lexmount_browser::cdp::Cdp::connect_to_target(ws_url, page_id)`.
`Cdp::connect(ws_url)` retains its existing default behavior.

### Local regression tests

```bash
cargo test --all-targets --locked
# Optional: use a local Chrome/Chromium executable, including chrome-headless-shell.
BROWSER_CLI_TEST_CHROME=/path/to/chrome cargo test --locked --test page_targets_browser -- --ignored --nocapture
```

In PowerShell, set `$env:BROWSER_CLI_TEST_CHROME` to the executable path before
running the same `cargo test` command. The opt-in test launches a separate
headless profile and loopback-only fixtures; it does not use a Lexmount account,
real websites, or an existing browser profile. The default suite exercises all
action routes and failure/no-fallback behavior with deterministic CDP fixtures.

## Agent Skill package

The publishable Skill is in `skills/lexmount-browser`. Build a deterministic ZIP:

```bash
./scripts/package-skill.sh
```

The ZIP contains `SKILL.md`, references, and platform bootstrap scripts at its
archive root. Native executables are published separately and are not placed in
the Skill ZIP. On first use, the matching bootstrap script downloads the pinned
release from Tencent Cloud COS and verifies its SHA-256 digest. Set
`LEXMOUNT_BROWSER_CLI_VERSION` or `LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL` only
when testing a different published release or mirror.

Updating the Skill files does not replace an existing Skill-local executable.
After the pinned release is available, an authorized upgrade can rerun the
matching bundled bootstrap script, then verify `browser-cli version` and
`browser-cli action --help`. If the release is not available, report that
dependency rather than substituting an older binary for a task needing the new
feature. Release tags must match the Cargo and bootstrap versions; never
overwrite an existing release with changed binaries.

### Release checklist

1. In a reviewed PR, update the package version in `Cargo.toml`, the
   `lexmount-browser` entry in `Cargo.lock`, and the defaults in both
   `skills/lexmount-browser/scripts/bootstrap.ps1` and `bootstrap.sh`.
2. Run `.github/scripts/test-release-version.ps1` with Windows PowerShell 5.1
   or PowerShell 7, then `.github/scripts/verify-release-version.ps1 -ReleaseTag
   v1.2.3` (substitute the intended version). Complete CI and merge the PR.
3. Create the matching tag **on that merged commit**. Typing a new tag or
   release title in GitHub does not update any source version. The release
   workflow rejects inconsistent versions before building, signing or uploading.
4. Wait for every build and the publish job. Each platform's compiled binary
   must report the tag's version before packaging. Verify the downloaded asset's
   checksum and `browser-cli version`; the Skill ZIP must pin the same version.

The published `v1.2.2` assets include the error-reporting fixes, but were built
with Cargo version `1.2.1` and Skill bootstrap defaults `1.2.1`. They are
misversioned; use the corrected `v1.2.3` release once published. Do not retag or
overwrite `v1.2.2`: consumers may already have its original files and checksums.

Agents resolve bundled scripts and binaries from the directory containing the
loaded `SKILL.md`: Codex uses the absolute source path supplied in the Skill
metadata, Claude Code uses `${CLAUDE_SKILL_DIR}`, and WorkBuddy/CodeBuddy uses
`${CODEBUDDY_SKILL_DIR}`. These are host-level Skill locators, not installation
or download inputs. Once started, the bootstrap and doctor scripts also locate
the Skill directory from their own path.

Published binaries include macOS arm64, Windows x64, and static Linux x64. The
macOS binary is signed with a Developer ID Application certificate,
hardened-runtime enabled, and accepted by Apple's notarization service before
it is published. macOS Intel remains an unsupported release platform.

The release workflow reads the signing certificate and notarization credentials
from the `macos-release` GitHub environment. It requires
`MACOS_DEVELOPER_ID_APPLICATION_P12_BASE64`,
`MACOS_DEVELOPER_ID_P12_PASSWORD`, `APPLE_NOTARY_APPLE_ID`,
`APPLE_NOTARY_TEAM_ID`, and `APPLE_NOTARY_APP_PASSWORD`.

The publish job uploads all three platform binaries and their checksum manifest to
Tencent Cloud COS through the `cos-release` GitHub environment. It requires
`TENCENT_CLOUD_SECRET_ID` and `TENCENT_CLOUD_SECRET_KEY` secrets plus
`COS_BUCKET`, `COS_REGION`, `COS_PUBLIC_BASE_URL`, and `COS_OBJECT_PREFIX`
variables.
