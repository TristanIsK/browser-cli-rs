# Command reference

Every command returns a JSON object with `ok` and either `data` or `error`.
The examples use `browser-cli` as shorthand for the Skill-local binary resolved
from the directory containing `SKILL.md`; invoke that binary by its absolute path.

```text
browser-cli doctor
browser-cli auth status
browser-cli auth login [--client-name "NAME"]

browser-cli session create [--browser-mode normal|light]
  [--context-id ID --context-mode read_write|read_only]
  [--downloads] [--recording] [--window-size 1920,1080]
  [--official-proxy | --local-proxy-tunnel-id TUNNEL_ID]
browser-cli tunnel open [--duration SECONDS]
browser-cli tunnel status --tunnel-id TUNNEL_ID
browser-cli tunnel close --tunnel-id TUNNEL_ID

browser-cli session list [--status active]
browser-cli session get --session-id ID
browser-cli session targets --session-id ID
browser-cli session keepalive --session-id ID [--duration 60]
browser-cli session close --session-id ID
browser-cli session downloads list --session-id ID
browser-cli session downloads get --session-id ID --download-id ID --output FILE
browser-cli session downloads archive --session-id ID --output FILE
browser-cli session downloads delete --session-id ID --yes

browser-cli context create [--description TEXT] [--metadata-json JSON]
browser-cli context list [--status available|locked] [--limit 20]
browser-cli context get --context-id ID
browser-cli context fork --context-id ID
browser-cli context delete --context-id ID --yes
browser-cli context force-release --context-id ID --yes

browser-cli action open-url --session-id ID --url URL
browser-cli action snapshot --session-id ID
browser-cli action wait-selector --session-id ID --selector CSS
browser-cli action wait-text --session-id ID --text "Saved" [--selector CSS]
browser-cli action wait-text --session-id ID --text "Saved" [--selector CSS] --exact
browser-cli action click --session-id ID --selector CSS
browser-cli action fill --session-id ID --selector CSS --value TEXT
browser-cli action screenshot --session-id ID --path FILE [--full-page]
browser-cli action pdf --session-id ID --path FILE [--print-background]
browser-cli action eval --session-id ID --expression JS
browser-cli action raw --session-id ID --method CDP_METHOD --params-json JSON
```

## Page selection

Introduced in 1.2.0; requires a binary whose `browser-cli action --help` lists
`--target-id`. The published 1.1.15 binary does not include this feature.
Check the actual Skill-local binary, not just the version of these instructions.

For an authorized upgrade, rerun the matching Skill-local bootstrap script only
after its pinned release assets are available, then verify `version` and
`action --help`. A merged PR or a newer Skill file does not publish or replace
the binary. If the release is unavailable or the upgrade is not authorized,
report the dependency or capability limitation; do not send unsupported flags
or drop the target selection to continue against a different page.

Every `action` above accepts optional `--target-id PAGE_ID`, before or after the
action subcommand. It selects an existing page inside `--session-id`; it is not
a replacement for the browser session ID. JSON output shapes are unchanged.

```text
browser-cli session targets --session-id SESSION_ID
browser-cli action snapshot --session-id SESSION_ID --target-id PAGE_ID
browser-cli action click --session-id SESSION_ID --target-id PAGE_ID --selector CSS
browser-cli session targets --session-id SESSION_ID
browser-cli action wait-selector --session-id SESSION_ID --target-id NEW_PAGE_ID --selector CSS
browser-cli action snapshot --session-id SESSION_ID --target-id NEW_PAGE_ID
```

Use the page entry's `id` in the DevTools target listing (`targetId` when using
CDP `Target.getTargets` directly). Match the expected URL/title and page type,
not the first/last position or an attached CDP `sessionId`. A popup may take time
to appear or navigate: refresh the listing within a bounded task timeout, then
wait for the required selector/text on the selected page. If several pages are
plausible, inspect them before choosing; do not blindly retry a state-changing
click. Pass the chosen ID on each subsequent action; selection is not persisted.

Without this option, the CLI keeps its original default: attach to the first
page returned by CDP, or create `about:blank` if no page exists. It does not
automatically follow a newly opened tab. A missing/closed explicit target fails
with `not_found`; a non-page target fails with `configuration_error`. A target
closed during attachment can produce `cdp_error`. The CLI never falls back to
another page or creates a page when an explicit target cannot be used. Re-list
targets and reassess the task instead of dropping `--target-id` to bypass errors.

Use temporary sessions for public browsing. Use a dedicated persistent Context per account or purpose; avoid sharing one read-write Context between parallel tasks.

`wait-text` uses case-insensitive normalized contains matching by default. Add
`--exact` only when the entire normalized text must match. The obsolete
`--match contains` form is not supported.

## Local network proxy (1.3.0)

Check that the installed binary lists `tunnel` and
`session create --local-proxy-tunnel-id`. Run `tunnel open` on the machine whose
local/VPN network is needed, keep that process running, and read its single
flushed JSON ready result. In a separate invocation create a normal session with
that tunnel ID, then use existing actions. The tunnel ID pins the region; use
the same project and API environment. It conflicts with `--official-proxy` and
a different `LEXMOUNT_REGION`. The server requires `custom_proxy` capability.

This grants selected cloud browser sessions access through that machine's local
DNS and TCP connections. It does not copy cookies or OS login, or tunnel UDP.
Do not start a local connector on a different machine from the intended network.
There is no cloud/direct fallback on tunnel failure. Reconnection terminates old
TCP streams; it never replays requests or browser actions. `tunnel status` checks
current server state; the initial ready output is not a continuous status feed.

Close browser sessions when done, then stop the foreground connector with Ctrl+C
(or `tunnel close --tunnel-id ID`). Closing the tunnel does not close sessions or
stop session billing. `--duration` limits tunnel lifetime and defaults to 0
(until interrupted or revoked). This process must remain alive between CLI
invocations, and does not keep browser sessions active by itself. Closing stdin
is not the shutdown signal; use Ctrl+C/termination or the close command.
