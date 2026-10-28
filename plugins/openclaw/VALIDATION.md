# Validation record

Target: OpenClaw 2026.10.8, Node.js 24.22.1, macOS arm64.

- ClawHub Plugin Inspector 0.4.3: no issues against the downloaded 2026.10.8 package.
- Skill creator quick validator: passed.
- npm artifact contains only this plugin, Skill, public metadata, documentation and license.
- Packed local installation: native plugin reports `loaded`; Skill reports
  `eligible`, `modelVisible` and `userInvocable`, with no missing requirements.
- Native `config set` site-switch round trip: China and back to international
  preserve disabled state, tool filter, OAuth metadata and request timeout.
- OpenClaw's native OAuth identity function generates different credential keys
  for the two full URLs and the same key when returning to a site.
- Actual Control UI: Settings / MCP shows the configured international URL.
  The site is edited as a URL, not through a custom dropdown.
- Both sites currently advertise PKCE S256, public client metadata documents,
  and the `browser.session:create` scope.
- International OAuth completed through the native OpenClaw login command and
  this public client registration; the host confirmed credentials saved.
- The real OpenClaw MCP runtime discovered 11 tools, created a temporary browser,
  opened Example.com, verified page text, received a PNG screenshot, requested
  close, and subsequently queried the session with status `closed`.
- ClawHub publication dry run accepted the package and exact source binding.

Still required before publication:

- ClawHub upload and review result.

This record does not claim Windows/Linux host acceptance, market publication,
China production OAuth, Control UI OAuth callback compatibility or an AI-driven
natural-language run.
