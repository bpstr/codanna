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
TAG = "v1.0.1"
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
        self.stub("gh", "raise SystemExit('GitHub CLI must not be used')\n")
        self.stub("curl", """import json, os, pathlib, shutil, sys
root = pathlib.Path(os.environ['FIXTURE_ROOT'])
args = sys.argv[1:]
with (root / 'requests').open('a') as log:
    log.write(repr(args) + '\\n')
if os.environ.get('FIXTURE_TRANSPORT_FAILURE'):
    sys.exit('fixture transport failure')
url = next(arg for arg in args if arg.startswith('https://'))
assert args[args.index('--proto') + 1] == '=https'
assert args[args.index('--proto-redir') + 1] == '=https'
destination = pathlib.Path(args[args.index('-o') + 1])
if url == 'https://api.github.com/repos/bpstr/codanna/releases/latest':
    assert args[args.index('-w') + 1] == '%{http_code}'
    status = os.environ.get('FIXTURE_HTTP_STATUS', '200')
    data = {'tag_name': os.environ['FIXTURE_TAG'], 'body': 'quoted "tag_name": "wrong"'}
    if status != '200':
        data = {'message': 'fixture HTTP failure'}
    destination.write_text(json.dumps(data, indent=2))
    print(status, end='')
else:
    tag = os.environ['FIXTURE_TAG']
    assert url.startswith('https://github.com/bpstr/codanna/releases/download/' + tag + '/')
    assert '-fsSL' in args
    source = root / 'assets' / url.rsplit('/', 1)[1]
    if not source.is_file():
        sys.exit('fixture asset unavailable')
    shutil.copyfile(source, destination)
""")
        self.archive()

    def stub(self, name, source):
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\n{source}")
        path.chmod(0o755)

    def archive(self, platform="linux-x64", member="codanna", symlink=False):
        directory = f"codanna-{self.env['FIXTURE_TAG'][1:]}-{platform}"
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

    def test_latest_stable_release_installs_verified_executable_without_gh(self):
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
        self.assertNotIn("api.github.com", (self.root / "requests").read_text())

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

    def test_transport_failure_preserves_installation(self):
        self.env["FIXTURE_TRANSPORT_FAILURE"] = "1"
        self.assert_failure_preserves_binary("cannot contact GitHub")

    def test_empty_release_tag_preserves_installation(self):
        self.env["FIXTURE_TAG"] = ""
        self.assert_failure_preserves_binary("missing a release tag")

    def test_unsupported_operating_system(self):
        self.env["FIXTURE_OS"] = "FreeBSD"
        self.assert_failure_preserves_binary("supported systems")
        self.assertFalse((self.root / "requests").exists())

    def test_linux_arm64_reports_missing_asset(self):
        self.env["FIXTURE_ARCH"] = "aarch64"
        self.assert_failure_preserves_binary("linux-arm64 binary")

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

    def test_no_stable_release_stops_before_asset_download(self):
        self.env["FIXTURE_HTTP_STATUS"] = "404"
        self.assert_failure_preserves_binary("no published stable release")
        self.assertNotIn("releases/download", (self.root / "requests").read_text())

    def test_api_rate_limit_is_reported(self):
        self.env["FIXTURE_HTTP_STATUS"] = "403"
        self.assert_failure_preserves_binary("denied or rate limited")

    def test_api_server_failure_is_reported(self):
        self.env["FIXTURE_HTTP_STATUS"] = "500"
        self.assert_failure_preserves_binary("lookup failed (HTTP 500)")

    def test_version_is_discovered_dynamically(self):
        self.env["FIXTURE_TAG"] = "v9.2.1"
        self.archive()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("v9.2.1", result.stdout)

    def test_explicit_published_prerelease_pin_skips_stable_lookup(self):
        self.env["FIXTURE_TAG"] = "v1.1.0-rc1"
        self.env["CODANNA_VERSION"] = "v1.1.0-rc1"
        self.archive()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("api.github.com", (self.root / "requests").read_text())


if __name__ == "__main__":
    unittest.main(verbosity=2)
