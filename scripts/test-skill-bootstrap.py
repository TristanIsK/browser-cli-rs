#!/usr/bin/env python3
"""Offline checks for platform selection and fail-closed CLI installation."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BOOTSTRAP = Path(__file__).resolve().parents[1] / "skills/lexmount-browser/scripts/bootstrap.sh"


class BootstrapTests(unittest.TestCase):
    def run_bootstrap(self, platform, checksum="valid", exit_code=0):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            shim = root / "shim"
            shim.mkdir()
            payload = f"#!/bin/sh\necho browser-cli-test\nexit {exit_code}\n".encode()
            (root / "payload").write_bytes(payload)
            digest = hashlib.sha256(payload).hexdigest()
            target = {"Darwin": "aarch64-apple-darwin", "Linux": "x86_64-unknown-linux-musl"}[platform]
            asset = f"browser-cli-v1.2.0-{target}"
            sums = f"{digest if checksum == 'valid' else '0' * 64}  {asset}\n"
            (root / "sums").write_text("" if checksum == "missing" else sums)
            (shim / "uname").write_text(
                f"#!/bin/sh\ncase $1 in -s) echo {platform};; -m) echo {'arm64' if platform == 'Darwin' else 'x86_64'};; esac\n"
            )
            (shim / "curl").write_text(
                f"#!{sys.executable}\n"
                "import os, pathlib, shutil, sys\n"
                "args = sys.argv[1:]\n"
                "assert args[:3] == ['--proto', '=https', '--tlsv1.2']\n"
                "url = next(a for a in args if a.startswith('https://'))\n"
                "root = pathlib.Path(os.environ['TEST_ROOT'])\n"
                "with (root / 'requests').open('a') as f: f.write(url + '\\n')\n"
                "shutil.copyfile(root / ('sums' if url.endswith('/SHA256SUMS') else 'payload'), args[args.index('-o') + 1])\n"
            )
            for path in shim.iterdir():
                path.chmod(0o755)
            env = dict(os.environ, PATH=f"{shim}:{os.environ['PATH']}", TEST_ROOT=tmp,
                       LEXMOUNT_BROWSER_CLI_VERSION="1.2.0",
                       LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL="https://downloads.example.invalid",
                       LEXMOUNT_BROWSER_CLI_INSTALL_DIR=str(root / "installed"))
            result = subprocess.run(["sh", str(BOOTSTRAP)], env=env, capture_output=True, text=True)
            self.assertIn(f"/v1.2.0/{asset}\n", (root / "requests").read_text())
            installed = root / "installed/browser-cli"
            if checksum != "valid":
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(installed.exists())
            elif exit_code:
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Installed browser-cli to", result.stdout)
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(installed.read_bytes(), payload)
                self.assertIn("browser-cli-test", result.stdout)

    def test_platforms_and_failure_paths(self):
        for platform in ("Darwin", "Linux"):
            for checksum in ("valid", "missing", "mismatch"):
                with self.subTest(platform=platform, checksum=checksum):
                    self.run_bootstrap(platform, checksum)
            with self.subTest(platform=platform, version_failure=True):
                self.run_bootstrap(platform, exit_code=7)


if __name__ == "__main__":
    unittest.main()
