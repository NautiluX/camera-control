# SPDX-License-Identifier: Apache-2.0
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import release


class ReleaseTests(unittest.TestCase):
    def test_version_sequence(self):
        self.assertEqual(release.next_version('0.1.0', None), '0.1.0')
        self.assertEqual(release.next_version('0.1.0', 'v0.1.9'), '0.1.10')
        self.assertEqual(release.next_version('0.2.0', 'v0.1.9'), '0.2.0')
        self.assertEqual(release.next_version('0.1.0', 'v1.0.0'), '1.0.1')

    def test_invalid_version(self):
        for version in ['1.0', '1.0.0-beta', '1.0.0\n', 'bad']:
            with self.assertRaises(ValueError):
                release.parse_version(version)

    def test_stamp_changes_only_root_version(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path.cwd()
            try:
                os.chdir(directory)
                Path('Cargo.toml').write_text('[package]\nname = "camera-control"\nversion = "0.1.0"\n\n[dependencies]\nexample = "0.1.0"\n')
                Path('Cargo.lock').write_text('[[package]]\nname = "camera-control"\nversion = "0.1.0"\n\n[[package]]\nname = "example"\nversion = "0.1.0"\n')
                release.stamp('0.1.1')
                self.assertIn('version = "0.1.1"', Path('Cargo.toml').read_text())
                self.assertIn('example = "0.1.0"', Path('Cargo.toml').read_text())
                self.assertIn('name = "example"\nversion = "0.1.0"', Path('Cargo.lock').read_text())
            finally:
                os.chdir(original)

    def test_api_failure_is_not_first_release(self):
        with patch('release.subprocess.run') as run:
            run.return_value.returncode = 1
            run.return_value.stdout = '{}'
            run.return_value.stderr = 'HTTP 403'
            with self.assertRaises(RuntimeError):
                release.latest_release('owner/repo')
