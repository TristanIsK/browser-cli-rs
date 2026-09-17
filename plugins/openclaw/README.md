# OpenClaw plugin package

The package root is the repository root (`.`). The root manifests load the existing `skills/lexmount-browser/` directory without moving or copying its source. The entry module needs no hooks: the shared Skill invokes the released browser CLI.

Build the upload artifact from an exact Git commit:

```sh
mkdir -p dist
npm pack --ignore-scripts --pack-destination dist
```

The npm file allowlist includes only the plugin wrapper, Skill, README and MIT license. It excludes Rust sources, downloaded binaries, build output and credentials. This package retains the repository's MIT license; it does not change the CLI license or grant cloud-service access.

Check `npm pack --dry-run --json`, run `clawhub package validate .`, and run a source-bound `clawhub package publish . --dry-run` before publishing. Use the actual GitHub repository containing the commit, its full commit SHA, and package path `.`. The ClawHub owner must match the npm scope; the current candidate uses `@tristanisk`.

Install the generated archive with `openclaw plugins install /absolute/path/to/package.tgz`. Open a fresh conversation and ask to open a webpage in the LexMount cloud browser, read its title, provide a screenshot, and close the temporary session. Service authorization is separate from plugin installation.

Bootstrap supports macOS arm64, Linux x86_64 and Windows x64. Run `python3 scripts/test-skill-bootstrap.py` for offline POSIX bootstrap checks. Windows real-host verification remains pending. Prior 1.1.18 acceptance does not certify this 1.2.0 package; validate the exact release artifact before publishing.
