# Installation and security review

This Skill uses local native code to control remote LexMount browser sessions.
Bootstrap is an explicit first-use installation step, not an npm install hook.
No elevated privileges are needed. The plugin entry registers no tools or hooks.

## Auditable release

Pinned CLI: [v1.2.3](https://github.com/lexmount/browser-cli-rs/releases/tag/v1.2.3).
Source: [release source tree](https://github.com/lexmount/browser-cli-rs/tree/v1.2.3).
Build: [.github/workflows/release.yml](https://github.com/lexmount/browser-cli-rs/blob/v1.2.3/.github/workflows/release.yml).
Credential implementation: [src/auth.rs](https://github.com/lexmount/browser-cli-rs/blob/v1.2.3/src/auth.rs).
Network configuration: [src/client.rs](https://github.com/lexmount/browser-cli-rs/blob/v1.2.3/src/client.rs).

The following hashes match the official GitHub release asset digests. Both installers
pin the applicable digest locally; they do not trust a checksum downloaded beside
the executable. Source/version/path environment overrides fail before network access.

| Target | File | SHA-256 |
| --- | --- | --- |
| aarch64-apple-darwin | browser-cli-v1.2.3-aarch64-apple-darwin | `85f7adabaf2599ab9d531b4c801c2648b85903671ec28a842033b0c3e1941b17` |
| x86_64-unknown-linux-musl | browser-cli-v1.2.3-x86_64-unknown-linux-musl | `35d6d6dbd0d81fda9d81531f85b009bfd2e7a62cbd3703f1d89f8667552aced3` |
| x86_64-pc-windows-msvc.exe | browser-cli-v1.2.3-x86_64-pc-windows-msvc.exe | `60c8fd5c9d501de5224363e08fa55022fa3af68fc3fcfef10be7609ccb4d7bae` |

Distribution prefix: `https://cli-bin-1377899528.cos.ap-nanjing.myqcloud.com/releases/browser-cli/v1.2.3/`.
Full download URLs are this prefix plus the exact filenames above. Hash pinning detects
changed downloads, but still requires trusting this publisher and reviewed native code;
it is not a claim of a reproducible build or a complete binary security audit.

## Access scope

- Network: release download at installation; configured LexMount API, approval page
  and returned CDP WebSocket at runtime; target websites run inside the cloud browser.
- Local writes: temporary download directory, Skill-local `bin/`, own CLI authorization
  file, and user-requested output files. No system-wide installation or permission edits.
- Tools: host read and command execution under existing host policy. OpenClaw Skills
  do not grant permissions; no invented `allowed-tools` field is presented as enforcement.
- Credentials: user-approved LexMount scope; no SSH/AWS or unrelated credential collection.
- External effects: purchases, publishing, destructive commands and account changes
  require explicit approval for the concrete action. Delete/force-release warnings are
  adjacent to the command reference, not inferred from a `--yes` flag.

## Response to the 1.2.0 audit

Download-and-execute is now disclosed before installation and integrity is pinned
independently of the download server. The three incomplete-artifact findings require
review of the release/source links above; no claim that the scanner inspected them.
The credential warning cited our own login-file documentation, not evidence of unrelated
credential harvesting. The high-impact-operation warning is addressed by explicit
approval rules in both the Skill and command/auth references. Rescan the new package;
this note does not instruct a reviewer to suppress findings or assume approval.
