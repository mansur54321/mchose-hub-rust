import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class InstallLocalTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.project = self.base / "project"
        self.home = self.base / "home"
        self.data = self.home / ".local/share"
        self.launcher = self.home / ".local/bin/mchose-native"
        self.alias = self.home / ".local/bin/mchose-hub-rust"
        self.desktop = self.data / "applications/mchose-native.desktop"
        self.icon = self.data / "icons/hicolor/512x512/apps/mchose-native.png"
        self.binary = self.project / "target/release/mchose-hub-rust"
        self.script = self.project / "scripts/install-local.py"
        self.script.parent.mkdir(parents=True)
        shutil.copyfile(Path(__file__).resolve().parents[1] / "scripts/install-local.py", self.script)
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        (self.project / "assets").mkdir()
        (self.project / "assets/mchose-icon.png").write_bytes(b"project icon")
        self.launcher.parent.mkdir(parents=True)

    def install(self, success=True):
        result = subprocess.run(
            [sys.executable, str(self.script)],
            env={**os.environ, "HOME": str(self.home), "XDG_DATA_HOME": str(self.data)},
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def old_desktop(self):
        self.desktop.parent.mkdir(parents=True)
        self.desktop.write_text(
            '[Desktop Entry]\nType=Application\nName=MCHOSE Native\n'
            f'Exec="{self.launcher}"\nStartupWMClass=mchose-native\n'
            'Icon=mchose-native\nX-MCHOSE-Native-Managed=true\nX-Custom=preserve\n'
        )

    def assert_installed(self, target=None):
        target = target or self.binary
        for path in (self.launcher, self.alias):
            self.assertTrue(path.is_symlink())
            self.assertEqual(os.readlink(path), str(target))
        self.assertIn("Name=MCHOSE Hub Rust\n", self.desktop.read_text())
        self.assertIn("StartupWMClass=mchose-native\n", self.desktop.read_text())
        self.assertEqual(self.icon.read_bytes(), b"project icon")

    def test_fresh_install_and_repeat_preserve_files(self):
        self.install()
        self.assert_installed()
        before = {p: p.lstat().st_ino for p in (self.launcher, self.alias, self.desktop, self.icon)}
        self.install()
        self.assert_installed()
        self.assertEqual(before, {p: p.lstat().st_ino for p in before})

    def test_migrates_owned_old_launcher_and_desktop_preserving_profiles(self):
        old_binary = self.project / "target/release/mchose-native"
        old_binary.write_text("old binary")
        self.launcher.symlink_to(old_binary)
        self.old_desktop()
        self.icon.parent.mkdir(parents=True)
        self.icon.write_bytes(b"old managed icon")
        profile = self.data / "mchose-native/profiles/example.json"
        profile.parent.mkdir(parents=True)
        profile.write_bytes(b"user profile")
        self.install()
        self.assert_installed()
        self.assertIn("X-Custom=preserve\n", self.desktop.read_text())
        self.assertEqual(profile.read_bytes(), b"user profile")
        self.assertEqual(old_binary.read_text(), "old binary")

    def test_migrates_owned_broken_old_launcher(self):
        self.launcher.symlink_to(self.project / "target/release/mchose-native")
        self.install()
        self.assert_installed()

    def test_packaged_binary_is_supported(self):
        packaged = self.project / "mchose-hub-rust"
        self.binary.rename(packaged)
        self.launcher.symlink_to(self.project / "mchose-native")
        self.install()
        self.assert_installed(packaged)

    def test_unrelated_regular_launcher_is_preserved(self):
        self.launcher.write_bytes(b"unrelated command")
        self.install(success=False)
        self.assertEqual(self.launcher.read_bytes(), b"unrelated command")
        self.assertFalse(self.alias.exists())
        self.assertFalse(self.desktop.exists())

    def test_unrelated_symlink_is_preserved(self):
        target = self.base / "other-program"
        self.launcher.symlink_to(target)
        self.install(success=False)
        self.assertEqual(os.readlink(self.launcher), str(target))
        self.assertFalse(self.alias.exists())

    def test_alias_conflict_does_not_migrate_old_launcher(self):
        old = self.project / "target/release/mchose-native"
        self.launcher.symlink_to(old)
        self.alias.write_bytes(b"unrelated alias")
        self.install(success=False)
        self.assertEqual(os.readlink(self.launcher), str(old))
        self.assertEqual(self.alias.read_bytes(), b"unrelated alias")
        self.assertFalse(self.desktop.exists())

    def test_unrelated_desktop_is_preserved(self):
        self.desktop.parent.mkdir(parents=True)
        self.desktop.write_bytes(b"[Desktop Entry]\nName=Other application\n")
        before = self.desktop.read_bytes()
        self.install(success=False)
        self.assertEqual(self.desktop.read_bytes(), before)
        self.assertFalse(self.launcher.exists())

    def test_unrelated_icon_is_preserved(self):
        self.icon.parent.mkdir(parents=True)
        self.icon.write_bytes(b"unrelated icon")
        self.install(success=False)
        self.assertEqual(self.icon.read_bytes(), b"unrelated icon")
        self.assertFalse(self.launcher.exists())


if __name__ == "__main__":
    unittest.main()
