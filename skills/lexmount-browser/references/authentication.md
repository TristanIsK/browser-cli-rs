# Authentication

Preferred login:

```text
browser-cli auth login [--client-name "NAME"]
```

The default client name is `Agent`. Quote and pass the current Agent's
user-facing name when available; for example, WorkBuddy can pass
`--client-name "WorkBuddy"`.

The CLI binds a random loopback port on `127.0.0.1`, creates a PKCE verifier and state, opens the Lexmount approval page, exchanges the returned one-time code, and stores the scoped credential in:

```text
~/.config/lexmount/browser-cli/credentials.json
```

The file is mode `0600` on Unix. The CLI redacts the API key from all JSON output.

For managed environments, the SDK also accepts `LEXMOUNT_API_KEY`, `LEXMOUNT_PROJECT_ID`, optional `LEXMOUNT_BASE_URL`, and optional `LEXMOUNT_REGION`. Do not ask users to paste secret values into an Agent chat.

Use `browser-cli auth logout` to remove only the local credential file. Environment variables are managed outside the CLI.

## Credential boundary

The path above belongs only to the user's LexMount CLI authorization. Do not read,
search, copy or upload SSH keys, cloud-provider credentials, browser profile stores,
or unrelated application credentials. Agent tools must use `auth status` / `doctor`
without printing credential contents. The CLI sends the scoped authorization only
to its configured LexMount service; do not override the API destination for this Skill.
See [security.md](security.md) for the source files reviewers can inspect.

`auth logout` deletes local LexMount authorization and requires an explicit logout
request; it is not routine browser-session cleanup. Confirm destructive website or
Context operations separately, even when authentication already succeeded.
