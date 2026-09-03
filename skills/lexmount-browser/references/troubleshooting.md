# Troubleshooting

Run `browser-cli doctor` first and use the failed check's message.

- `configuration_error`: run `browser-cli auth login`, or verify the managed environment contains both required variables.
- `authentication_error`: the credential was rejected; log out and authorize again.
- `conflict`: a read-write Context is already locked. Use another Context, wait for the active session, or use read-only mode. Force-release only after confirming the session is dead.
- `timeout`: inspect session status and network access, then retry with a larger timeout.
- `cdp_error`: verify the session is active, inspect `session targets`, and take a snapshot before retrying the action.
- Skill root unknown: resolve the directory containing the loaded `SKILL.md` with the current host's locator: Codex supplies its absolute source path in the Skill metadata, Claude Code provides `${CLAUDE_SKILL_DIR}`, and WorkBuddy/CodeBuddy provides `${CODEBUDDY_SKILL_DIR}`. Do not infer it from the working directory or search the user's home directory.
- command not found after bootstrap: invoke `"<skill-root>/bin/browser-cli"` on macOS arm64 or Linux x64, or `& "<skill-root>\bin\browser-cli.exe"` in Windows PowerShell; no PATH change or restart is required.

Always close a newly created temporary session when abandoning a failed task.
