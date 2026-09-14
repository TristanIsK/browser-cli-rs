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

## Select a page in a multi-tab session

Explicit page selection is introduced in version 1.2.0. Check that the installed
binary's `browser-cli action --help` lists `--target-id`; the published 1.1.15
binary does not have it. The package version and both bootstrap scripts target
1.2.0 together. Merging or building this source does not publish release assets:
bootstrap can install 1.2.0 only after its binaries and checksums are published
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
