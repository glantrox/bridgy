import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

from package_release import UNIX_TARGETS, WINDOWS_TARGET, digest, package
from render_packages import render


class Packages(unittest.TestCase):
    def test_archives_and_manifests_use_real_hashes_and_command_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "input"
            binary.write_bytes(b"test executable")
            artifacts = root / "dist"
            for target in (*UNIX_TARGETS, WINDOWS_TARGET):
                package(binary, target, artifacts)
            for target in UNIX_TARGETS:
                archive = artifacts / f"bridgy-{target}.tar.gz"
                with tarfile.open(archive) as bundle:
                    self.assertEqual(bundle.getnames(), ["bridgy"])
                    self.assertEqual(bundle.extractfile("bridgy").read(), binary.read_bytes())
                    self.assertEqual(bundle.getmember("bridgy").mode & 0o777, 0o755)
            windows = artifacts / f"bridgy-{WINDOWS_TARGET}.zip"
            with zipfile.ZipFile(windows) as bundle:
                self.assertEqual(bundle.namelist(), ["bridgy.exe"])
                self.assertEqual(bundle.read("bridgy.exe"), binary.read_bytes())
            for checksum in artifacts.glob("*.sha256"):
                original = checksum.with_suffix("")
                self.assertEqual(checksum.read_text(), f"{digest(original)}  {original.name}\n")
            output = root / "packages"
            render("Example/bridgy", "v1.2.3", artifacts, output)
            formula = (output / "bridgy.rb").read_text()
            self.assertIn('bin.install "bridgy"', formula)
            for target in UNIX_TARGETS:
                self.assertIn(digest(artifacts / f"bridgy-{target}.tar.gz"), formula)
            manifest = (output / "winget/Example.Bridgy.installer.yaml").read_text()
            self.assertIn("PortableCommandAlias: bridgy", manifest)
            self.assertIn("RelativeFilePath: bridgy.exe", manifest)
            self.assertIn("NestedInstallerType: portable", manifest)
            self.assertIn(digest(windows), manifest)
            self.assertIn("https://github.com/Example/bridgy/releases/download/v1.2.3/", manifest)
            self.assertEqual(len(list((output / "winget").glob("*.yaml"))), 3)

    def test_no_placeholder_packages_without_release_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(FileNotFoundError):
                render("Example/bridgy", "v1.0.0", root, root / "packages")
            self.assertFalse((root / "packages").exists())
            with self.assertRaises(ValueError):
                render("invalid; repo", "v1.0.0", root, root)


if __name__ == "__main__":
    unittest.main()
