# Troubleshooting

Run `browser-cli doctor` first and use the failed check's message.

- `configuration_error`: read the message first. For an empty or non-page `--target-id`, inspect `session targets` and select a page; re-authentication will not fix page selection. For missing credentials, run `browser-cli auth login`, or verify the managed environment contains both required variables.
- `not_found` for a page target: it may have closed or belong to another browser session. Re-list `session targets` for the intended session and choose the correct page. Do not remove `--target-id` and accidentally retry against the default page.
- `authentication_error`: the credential was rejected; log out and authorize again.
- `conflict`: a read-write Context is already locked. Use another Context, wait for the active session, or use read-only mode. Force-release only after confirming the session is dead.
- `timeout`: inspect session status and network access, then retry with a larger timeout.
- `cdp_error`: verify the session is active and inspect `session targets`; a page can disappear between listing and attachment. Take a snapshot of the explicitly selected page before deciding whether to retry the action.
- Source URL unchanged after a click: the click may have opened a new tab. Inspect `session targets` and use `--target-id` for the intended new page; do not assume the click failed or that subsequent commands automatically follow it. See [page selection](commands.md#page-selection).
- Skill root unknown: resolve the directory containing the loaded `SKILL.md` with the current host's locator: Codex supplies its absolute source path in the Skill metadata, Claude Code provides `${CLAUDE_SKILL_DIR}`, and WorkBuddy/CodeBuddy provides `${CODEBUDDY_SKILL_DIR}`. Do not infer it from the working directory or search the user's home directory.
- command not found after bootstrap: invoke `"<skill-root>/bin/browser-cli"` on macOS arm64 or `& "<skill-root>\bin\browser-cli.exe"` in Windows PowerShell; no PATH change or restart is required.

Always close a newly created temporary session when abandoning a failed task.
