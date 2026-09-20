#!/bin/sh
set -eu

version="${LEXMOUNT_BROWSER_CLI_VERSION:-1.2.1}"
download_base_url="${LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL:-https://cli-bin-1377899528.cos.ap-nanjing.myqcloud.com/releases/browser-cli}"
repo="${download_base_url%/}/v${version}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  *) echo "Unsupported platform: $(uname -s) $(uname -m). This release supports macOS arm64 and Windows x86_64." >&2; exit 2 ;;
esac

asset="browser-cli-v${version}-${target}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM
curl --proto '=https' --tlsv1.2 -fsSL "$repo/$asset" -o "$tmp_dir/$asset"
curl --proto '=https' --tlsv1.2 -fsSL "$repo/SHA256SUMS" -o "$tmp_dir/SHA256SUMS"
expected="$(awk -v name="$asset" '$2 == name {print $1}' "$tmp_dir/SHA256SUMS")"
[ -n "$expected" ] || { echo "No checksum published for $asset" >&2; exit 3; }
actual="$(openssl dgst -sha256 "$tmp_dir/$asset" | awk '{print $NF}')"
[ "$expected" = "$actual" ] || { echo "SHA-256 mismatch for $asset" >&2; exit 4; }
skill_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
install_dir="${LEXMOUNT_BROWSER_CLI_INSTALL_DIR:-$skill_dir/bin}"
mkdir -p "$install_dir"
install -m 0755 "$tmp_dir/$asset" "$install_dir/browser-cli"
"$install_dir/browser-cli" version
echo "Installed browser-cli to $install_dir/browser-cli"
