"""Exercise the fork installer against local downloads and a disposable home."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


@unittest.skipUnless(os.name == "posix", "requires a POSIX shell")
class OrchaInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="orcha-installer-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "commands"
        self.bin.mkdir()
        self.home = self.root / "home"
        self.installed = self.home / ".local/bin/orcha"
        self.installed.parent.mkdir(parents=True)
        self.installed.write_bytes(b"previous app")
        self.payload = self.root / "payload"
        self.payload.write_text("#!/bin/sh\nexit 1\n")
        self.sums = self.root / "SHA256SUMS.txt"
        self.sums.write_text(hashlib.sha256(self.payload.read_bytes()).hexdigest() + "  orcha-macos-arm64\n")
        for name in ("awk", "cat", "chmod", "cp", "cut", "mkdir", "mktemp", "rm", "shasum"):
            command = shutil.which(name)
            self.assertIsNotNone(command, name)
            (self.bin / name).symlink_to(command)
        self.script("uname", '#!/bin/sh\ncase "$1" in -m) echo arm64 ;; *) echo Darwin ;; esac\n')
        self.script("git", "#!/bin/sh\nexit 0\n")
        self.script("xattr", "#!/bin/sh\nexit 0\n")
        self.script("python3", "#!/bin/sh\nexit 0\n")
        self.script("curl", '''#!/bin/sh
out=
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift ;; http*) url=$1 ;; esac
  shift
done
printf '%s\\n' "$url" >> "$REQUESTS"
case "$url" in
  */SHA256SUMS.txt) cp "$SUMS" "$out" ;;
  */orcha-macos-arm64) cp "$PAYLOAD" "$out" ;;
  *) exit 7 ;;
esac
''')
        real_mv = shutil.which("mv")
        self.script("mv", f'''#!/bin/sh
if [ "${{RENAME_FAIL:-}}" = 1 ]; then exit 1; fi
exec '{real_mv}' "$@"
''')
        self.extras = self.root / "extras"
        (self.extras / ".git").mkdir(parents=True)

    def script(self, name, contents):
        path = self.bin / name
        path.write_text(contents)
        path.chmod(0o755)

    def run_installer(self, **extra):
        return subprocess.run(
            ["/bin/sh", str(INSTALLER), "--no-office"],
            env={**os.environ, "PATH": str(self.bin), "HOME": str(self.home),
                 "ORCHA_DIR": str(self.extras), "ORCHA_INSTALL_TAG": "orcha-0.13.0",
                 "PAYLOAD": str(self.payload), "SUMS": str(self.sums),
                 "REQUESTS": str(self.root / "requests"), **extra},
            capture_output=True, text=True, timeout=10,
        )

    def test_checked_download_replaces_app_and_stays_on_the_pinned_release(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.installed.read_bytes(), self.payload.read_bytes())
        self.assertTrue(os.access(self.installed, os.X_OK))
        requests = (self.root / "requests").read_text().splitlines()
        self.assertEqual(len(requests), 2)
        self.assertTrue(all("/releases/download/orcha-0.13.0/" in url for url in requests))
        self.assertEqual(list(self.installed.parent.glob(".orcha-install.*")), [])

    def test_failed_rename_preserves_the_existing_app(self):
        result = self.run_installer(RENAME_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.installed.read_bytes(), b"previous app")
        self.assertEqual(list(self.installed.parent.glob(".orcha-install.*")), [])

    def test_a_bad_or_missing_checksum_keeps_the_existing_app(self):
        for sums in ("0" * 64 + "  orcha-macos-arm64\n", "0" * 64 + "  some-other-file\n"):
            with self.subTest(sums=sums):
                self.sums.write_text(sums)
                result = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("checksum", result.stdout)
                self.assertEqual(self.installed.read_bytes(), b"previous app")
                self.assertEqual(list(self.installed.parent.glob(".orcha-install.*")), [])


if __name__ == "__main__":
    unittest.main()
