#!/usr/bin/env python3
"""Offline installer regressions; no GitHub or model requests are made."""

import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[2] / "scripts/install.sh"
TAG = "v1.0.0-rc5"
PAYLOAD = b"#!/bin/sh\nprintf 'codanna fixture\\n'\n"


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="codanna-install-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.assets = self.root / "assets"
        self.install_dir = self.root / "installed with spaces"
        self.scratch = self.root / "scratch"
        for directory in (self.bin, self.assets, self.install_dir, self.scratch):
            directory.mkdir()
        self.env = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "HOME": str(self.root / "home"),
            "TMPDIR": str(self.scratch),
            "CODANNA_INSTALL_DIR": str(self.install_dir),
            "FIXTURE_ROOT": str(self.root),
            "FIXTURE_TAG": TAG,
            "FIXTURE_OS": "Linux",
            "FIXTURE_ARCH": "x86_64",
        }
        self.stub("uname", """import os, sys
print(os.environ['FIXTURE_OS' if sys.argv[1] == '-s' else 'FIXTURE_ARCH'])
""")
        self.stub("gh", """import os, pathlib, shutil, sys
root = pathlib.Path(os.environ['FIXTURE_ROOT'])
args = sys.argv[1:]
with (root / 'requests').open('a') as log:
    log.write(repr(args) + '\\n')
if os.environ.get('FIXTURE_AUTH_FAILURE'):
    sys.exit('fixture authentication failure')
if args[0] == 'api':
    assert args[1] == 'repos/bpstr/codanna/releases?per_page=100'
    assert args[2:] == ['--jq', '.[0].tag_name // empty']
    print(os.environ['FIXTURE_TAG'])
else:
    assert args[:2] == ['release', 'download']
    assert args[args.index('--repo') + 1] == 'bpstr/codanna'
    assert args[2] == os.environ['FIXTURE_TAG']
    destination = pathlib.Path(args[args.index('--dir') + 1])
    for i, arg in enumerate(args):
        if arg == '--pattern':
            source = root / 'assets' / args[i + 1]
            if not source.is_file():
                sys.exit('fixture asset unavailable')
            shutil.copyfile(source, destination / source.name)
""")
        self.archive()

    def stub(self, name, source):
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\n{source}")
        path.chmod(0o755)

    def archive(self, platform="linux-x64", member="codanna", symlink=False):
        directory = f"codanna-{TAG[1:]}-{platform}"
        self.package = self.assets / f"{directory}.tar.xz"
        with tarfile.open(self.package, "w:xz") as archive:
            entry = tarfile.TarInfo(f"{directory}/{member}")
            entry.mode = 0o644  # Installer must make the downloaded file executable.
            if symlink:
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/etc/passwd"
                archive.addfile(entry)
            else:
                entry.size = len(PAYLOAD)
                archive.addfile(entry, io.BytesIO(PAYLOAD))
        self.checksum = Path(f"{self.package}.sha256")
        self.checksum.write_text(hashlib.sha256(self.package.read_bytes()).hexdigest() + "\n")

    def run_installer(self):
        result = subprocess.run(
            ["sh", str(INSTALLER)], env=self.env, capture_output=True, text=True
        )
        self.assertEqual(list(self.scratch.iterdir()), [], result.stderr)
        self.assertEqual(list(self.install_dir.glob(".codanna-install.*")), [])
        return result

    def assert_failure_preserves_binary(self, message):
        binary = self.install_dir / "codanna"
        binary.write_bytes(b"existing installation")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assertEqual(binary.read_bytes(), b"existing installation")

    def test_latest_visible_release_installs_verified_executable(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = self.install_dir / "codanna"
        self.assertEqual(binary.read_bytes(), PAYLOAD)
        self.assertTrue(os.access(binary, os.X_OK))
        self.assertIn(TAG, result.stdout)
        self.assertIn('export PATH=', result.stdout)

    def test_pinned_version_skips_release_listing(self):
        self.env["CODANNA_VERSION"] = TAG[1:]
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("'api'", (self.root / "requests").read_text())

    def test_default_install_directory(self):
        del self.env["CODANNA_INSTALL_DIR"]
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((Path(self.env["HOME"]) / ".local/bin/codanna").read_bytes(), PAYLOAD)

    def test_checksum_mismatch_preserves_installation(self):
        self.checksum.write_text("0" * 64 + "\n")
        self.assert_failure_preserves_binary("checksum mismatch")

    def test_invalid_checksum_preserves_installation(self):
        self.checksum.write_text("g" * 64 + "\n")
        self.assert_failure_preserves_binary("invalid SHA-256 checksum")

    def test_missing_checksum_preserves_installation(self):
        self.checksum.unlink()
        self.assert_failure_preserves_binary("download failed")

    def test_missing_executable_preserves_installation(self):
        self.archive(member="wrong-name")
        self.assert_failure_preserves_binary("release archive does not contain")

    def test_symlink_executable_is_rejected(self):
        self.archive(symlink=True)
        self.assert_failure_preserves_binary("not a regular file")

    def test_authentication_failure_preserves_installation(self):
        self.env["FIXTURE_AUTH_FAILURE"] = "1"
        self.assert_failure_preserves_binary("gh auth login")

    def test_no_visible_releases_preserves_installation(self):
        self.env["FIXTURE_TAG"] = ""
        self.assert_failure_preserves_binary("no releases are visible")

    def test_unsupported_operating_system(self):
        self.env["FIXTURE_OS"] = "FreeBSD"
        self.assert_failure_preserves_binary("supported systems")
        self.assertFalse((self.root / "requests").exists())

    def test_linux_arm64_reports_missing_asset(self):
        self.env["FIXTURE_ARCH"] = "aarch64"
        self.assert_failure_preserves_binary("Linux ARM64")

    def test_invalid_pinned_tag_is_rejected_before_download(self):
        self.env["CODANNA_VERSION"] = "../../unexpected"
        self.assert_failure_preserves_binary("invalid release version")
        self.assertFalse((self.root / "requests").exists())

    def test_macos_arm64_uses_matching_archive(self):
        self.env["FIXTURE_OS"] = "Darwin"
        self.env["FIXTURE_ARCH"] = "arm64"
        self.archive(platform="macos-arm64")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("macos-arm64", result.stdout)

    def test_sha256sum_backend(self):
        self.stub("sha256sum", """import hashlib, pathlib, sys
path = pathlib.Path(sys.argv[1])
print(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + str(path))
""")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_install_destination_directory_is_preserved(self):
        destination = self.install_dir / "codanna"
        destination.mkdir()
        marker = destination / "user-file"
        marker.write_text("preserve")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("install destination is a directory", result.stderr)
        self.assertEqual(list(destination.iterdir()), [marker])

    def test_failed_final_rename_preserves_existing_binary(self):
        self.stub("mv", "import sys\nsys.exit('fixture rename failure')\n")
        self.assert_failure_preserves_binary("fixture rename failure")


if __name__ == "__main__":
    unittest.main(verbosity=2)
