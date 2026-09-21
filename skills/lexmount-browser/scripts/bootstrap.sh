#!/bin/sh
set -eu

# Release pins are reviewed with this package. Environment overrides cannot change trust.
if [ -n "${LEXMOUNT_BROWSER_CLI_VERSION:-}${LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL:-}${LEXMOUNT_BROWSER_CLI_INSTALL_DIR:-}" ]; then
  echo "Release overrides are disabled. Unset LEXMOUNT_BROWSER_CLI_VERSION, LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL and LEXMOUNT_BROWSER_CLI_INSTALL_DIR." >&2
  exit 2
fi
version="${LEXMOUNT_BROWSER_CLI_VERSION:-1.2.3}"
download_base_url="${LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL:-https://cli-bin-1377899528.cos.ap-nanjing.myqcloud.com/releases/browser-cli}"
repo="${download_base_url%/}/v${version}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin"; expected="85f7adabaf2599ab9d531b4c801c2648b85903671ec28a842033b0c3e1941b17" ;;
  Linux-x86_64) target="x86_64-unknown-linux-musl"; expected="35d6d6dbd0d81fda9d81531f85b009bfd2e7a62cbd3703f1d89f8667552aced3" ;;
  *) echo "Unsupported platform: $(uname -s) $(uname -m). This release supports macOS arm64, Linux x86_64 and Windows x86_64." >&2; exit 2 ;;
esac

asset="browser-cli-v${version}-${target}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL "$repo/$asset" -o "$tmp_dir/$asset"
actual="$(openssl dgst -sha256 "$tmp_dir/$asset" | awk '{print $NF}')"
[ "$expected" = "$actual" ] || { echo "SHA-256 mismatch for $asset" >&2; exit 4; }
skill_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
install_dir="${LEXMOUNT_BROWSER_CLI_INSTALL_DIR:-$skill_dir/bin}"
mkdir -p "$install_dir"
[ ! -L "$install_dir" ] && [ ! -L "$install_dir/browser-cli" ] || { echo "Refusing symlink installation target" >&2; exit 5; }
chmod 0755 "$tmp_dir/$asset"
"$tmp_dir/$asset" version
staged="$(mktemp "$install_dir/.browser-cli.XXXXXX")"
trap 'rm -rf "$tmp_dir"; rm -f "$staged"' EXIT INT TERM
install -m 0755 "$tmp_dir/$asset" "$staged"
mv -f "$staged" "$install_dir/browser-cli"
"$install_dir/browser-cli" version
echo "Installed browser-cli to $install_dir/browser-cli"
