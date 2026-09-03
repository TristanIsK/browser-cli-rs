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

## Agent Skill package

The publishable Skill is in `skills/lexmount-browser`. Build a deterministic ZIP:

```bash
./scripts/package-skill.sh
```

The ZIP contains `SKILL.md`, references, and bootstrap scripts for macOS arm64,
Linux x64, and Windows x64 at its archive root. Native executables are published
separately and are not placed in
the Skill ZIP. On first use, the matching bootstrap script downloads the pinned
release from Tencent Cloud COS and verifies its SHA-256 digest. Set
`LEXMOUNT_BROWSER_CLI_VERSION` or `LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL` only
when testing a different published release or mirror.

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
