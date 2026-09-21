#!/usr/bin/env python3
"""Offline regression: untrusted overrides/downloads never reach execution."""
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
OVERRIDES = ('LEXMOUNT_BROWSER_CLI_VERSION', 'LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL', 'LEXMOUNT_BROWSER_CLI_INSTALL_DIR')

class BootstrapSecurity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.skill = self.root / 'skill'
        shutil.copytree(ROOT / 'skills/lexmount-browser', self.skill, ignore=shutil.ignore_patterns('bin'))
        self.script = self.skill / 'scripts/bootstrap.sh'
        self.fake = self.root / 'fake'
        self.fake.mkdir()
        self.env = {k: v for k, v in os.environ.items() if k not in OVERRIDES}
        self.env.update(PATH=str(self.fake)+':'+os.environ['PATH'], TEST_ROOT=str(self.root), TEST_OS='Darwin', TEST_ARCH='arm64')
        self.executable('uname', 'case "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac')
        self.executable('curl', '''echo "$*" >> "$TEST_ROOT/requests"
while [ "$#" -gt 0 ]; do
  if [ "$1" = '-o' ]; then cp "$TEST_ROOT/payload" "$2"; exit; fi
  shift
done
exit 1''')
        self.payload = b'#!/bin/sh\necho executed >> "$TEST_ROOT/executed"\nexit 0\n'
        (self.root/'payload').write_bytes(self.payload)
        (self.skill/'bin').mkdir()
        self.installed = self.skill/'bin/browser-cli'
        self.installed.write_text('previous binary')

    def executable(self, name, code):
        p=self.fake/name
        p.write_text('#!/bin/sh\nset -eu\n'+code+'\n')
        p.chmod(0o755)

    def run_bootstrap(self):
        return subprocess.run(['sh',str(self.script)],env=self.env,capture_output=True,text=True)

    def test_environment_overrides_rejected_before_network(self):
        for key in OVERRIDES:
            with self.subTest(key=key):
                self.env[key]='https://attacker.invalid/replacement'
                result=self.run_bootstrap()
                self.assertNotEqual(result.returncode,0)
                self.assertIn('overrides are disabled',result.stderr)
                self.assertFalse((self.root/'requests').exists())
                self.assertFalse((self.root/'executed').exists())
                self.assertEqual(self.installed.read_text(),'previous binary')
                del self.env[key]

    def test_changed_binary_rejected_and_previous_install_preserved(self):
        result=self.run_bootstrap()
        self.assertNotEqual(result.returncode,0)
        self.assertIn('SHA-256 mismatch',result.stderr)
        self.assertFalse((self.root/'executed').exists())
        self.assertEqual(self.installed.read_text(),'previous binary')
        requests=(self.root/'requests').read_text()
        self.assertNotIn('SHA256SUMS',requests)
        self.assertIn('--proto-redir =https',requests)

    def test_pinned_payload_installs_on_both_posix_targets(self):
        # Replace the pin in a disposable script, not a production override hook.
        text=re.sub(r'expected="[0-9a-f]{64}"', 'expected="'+hashlib.sha256(self.payload).hexdigest()+'"',self.script.read_text())
        self.script.write_text(text)
        for system,arch,target in [('Darwin','arm64','aarch64-apple-darwin'),('Linux','x86_64','x86_64-unknown-linux-musl')]:
            with self.subTest(system=system):
                self.env.update(TEST_OS=system,TEST_ARCH=arch)
                result=self.run_bootstrap()
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(self.installed.read_bytes(),self.payload)
                self.assertIn('browser-cli-v1.2.3-'+target,(self.root/'requests').read_text())

    def test_symlink_destination_rejected(self):
        self.script.write_text(re.sub(r'expected="[0-9a-f]{64}"','expected="'+hashlib.sha256(self.payload).hexdigest()+'"',self.script.read_text()))
        outside=self.root/'outside'
        outside.write_text('do not overwrite')
        self.installed.unlink()
        self.installed.symlink_to(outside)
        self.assertNotEqual(self.run_bootstrap().returncode,0)
        self.assertEqual(outside.read_text(),'do not overwrite')
        self.assertFalse((self.root/'executed').exists())

if __name__ == '__main__':
    unittest.main()
