from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "copy-swag-sorotte-certs.sh"
MANIFEST_SCHEMA = "sorotte-tls-bundle-v1"
MEMBER_NAMES = ("privkey.pem", "cert.pem", "chain.pem")


def posix_command_path(path: Path) -> str:
    return path.resolve().as_posix()


def find_posix_shell() -> str | None:
    shell = shutil.which("sh")
    if shell is not None:
        return shell
    if os.name == "nt":
        git_shell = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git/bin/sh.exe"
        if git_shell.is_file():
            return str(git_shell)
    return None


def write_fixture_command(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8", newline="\n")
    path.chmod(0o755)


def expected_member(contents: bytes) -> dict[str, object]:
    return {
        "length": len(contents),
        "sha256": hashlib.sha256(contents).hexdigest(),
    }


class AtomicTlsPublisherTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.shell = find_posix_shell()
        if cls.shell is None:
            raise unittest.SkipTest("a POSIX shell is required for publisher integration tests")

    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.source = self.root / "source"
        self.archive = self.root / "archive"
        self.target = self.root / "target"
        self.fake_bin = self.root / "fake-bin"
        self.lineage_file = self.root / "lineage-number"
        self.source.mkdir()
        self.archive.mkdir()
        self.fake_bin.mkdir()
        self.next_lineage = 1

        write_fixture_command(
            self.fake_bin / "id",
            '#!/bin/sh\n[ "${1:-}" = "-u" ] && { printf "0\\n"; exit 0; }\nexit 64\n',
        )
        for command in ("chown", "chmod", "sync"):
            write_fixture_command(self.fake_bin / command, "#!/bin/sh\nexit 0\n")
        write_fixture_command(
            self.fake_bin / "readlink",
            """#!/bin/sh
last=
for argument in "$@"; do
    last="$argument"
done
filename="${last##*/}"
prefix="${filename%.pem}"
lineage="$(cat "$SOROTTE_TEST_LINEAGE_FILE")"
if [ "$prefix" = "chain" ] && [ -n "${SOROTTE_TEST_CHAIN_LINEAGE:-}" ]; then
    lineage="$SOROTTE_TEST_CHAIN_LINEAGE"
fi
printf '%s/%s%s.pem\\n' "$SOROTTE_TEST_ARCHIVE_DIR" "$prefix" "$lineage"
""",
        )
        write_fixture_command(
            self.fake_bin / "mv",
            """#!/bin/sh
last=
for argument in "$@"; do
    last="$argument"
done
if [ -n "${SOROTTE_TEST_FAIL_SELECTOR_MARKER:-}" ]; then
    case "$last" in
        */current.json)
            if [ ! -e "$SOROTTE_TEST_FAIL_SELECTOR_MARKER" ]; then
                : > "$SOROTTE_TEST_FAIL_SELECTOR_MARKER"
                exit 73
            fi
            ;;
    esac
fi
exec /usr/bin/mv "$@"
""",
        )

        self.environment = os.environ.copy()
        self.environment.update(
            {
                "SOROTTE_CERT_DOMAIN": "example.test",
                "SWAG_CERT_DIR": posix_command_path(self.source),
                "SOROTTE_TEST_ARCHIVE_DIR": posix_command_path(self.archive),
                "SOROTTE_TEST_LINEAGE_FILE": posix_command_path(self.lineage_file),
                "SOROTTE_TLS_DIR": posix_command_path(self.target),
                "SOROTTE_UID": "10001",
                "SOROTTE_GID": "10001",
            }
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_source_generation(self, marker: bytes) -> dict[str, bytes]:
        lineage = self.next_lineage
        self.next_lineage += 1
        self.lineage_file.write_text(str(lineage), encoding="ascii")
        members = {
            "privkey.pem": b"private-key-" + marker + b"\n",
            "cert.pem": b"leaf-certificate-" + marker + b"\n",
            "chain.pem": b"certificate-chain-" + marker + b"\n",
        }
        for filename, contents in members.items():
            (self.source / filename).write_bytes(contents)
            prefix = filename.removesuffix(".pem")
            (self.archive / f"{prefix}{lineage}.pem").write_bytes(contents)
        return members

    def publish(
        self,
        *,
        expected_exit: int = 0,
        fail_selector_marker: Path | None = None,
        environment_overrides: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        environment = self.environment.copy()
        if fail_selector_marker is not None:
            environment["SOROTTE_TEST_FAIL_SELECTOR_MARKER"] = posix_command_path(
                fail_selector_marker
            )
        if environment_overrides is not None:
            environment.update(environment_overrides)
        completed = subprocess.run(
            [
                self.shell,
                "-c",
                (
                    'fixture_bin="$1"; '
                    'if command -v cygpath >/dev/null 2>&1; then '
                    'fixture_bin="$(cygpath -u "$fixture_bin")"; fi; '
                    'PATH="$fixture_bin:$PATH"; export PATH; exec sh "$2"'
                ),
                "atomic-tls-publisher-test",
                posix_command_path(self.fake_bin),
                posix_command_path(SCRIPT_PATH),
            ],
            cwd=REPO_ROOT,
            env=environment,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(
            completed.returncode,
            expected_exit,
            f"publisher exit mismatch\nstdout:\n{completed.stdout}\nstderr:\n{completed.stderr}",
        )
        return completed

    def current_manifest(self) -> dict[str, object]:
        return json.loads((self.target / "current.json").read_text(encoding="utf-8"))

    def assert_selected_generation(
        self, manifest: dict[str, object], expected: dict[str, bytes]
    ) -> Path:
        self.assertEqual(manifest["schema"], MANIFEST_SCHEMA)
        generation = manifest["generation"]
        self.assertIsInstance(generation, str)
        assert isinstance(generation, str)
        self.assertRegex(generation, re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,126}[A-Za-z0-9]$"))
        generation_root = self.target / "generations" / generation
        self.assertTrue(generation_root.is_dir())
        members = manifest["members"]
        self.assertIsInstance(members, dict)
        assert isinstance(members, dict)
        self.assertEqual(set(members), set(MEMBER_NAMES))
        for filename in MEMBER_NAMES:
            self.assertEqual(members[filename], expected_member(expected[filename]))
            self.assertEqual((generation_root / filename).read_bytes(), expected[filename])
        return generation_root

    def test_successive_publications_are_immutable_authenticated_and_atomic(self) -> None:
        generation_a = self.write_source_generation(b"A")
        self.publish()
        manifest_a = self.current_manifest()
        generation_a_root = self.assert_selected_generation(manifest_a, generation_a)
        generation_a_bytes = {
            filename: (generation_a_root / filename).read_bytes() for filename in MEMBER_NAMES
        }

        generation_b = self.write_source_generation(b"B")
        self.publish()
        manifest_b = self.current_manifest()
        generation_b_root = self.assert_selected_generation(manifest_b, generation_b)

        self.assertNotEqual(manifest_a["generation"], manifest_b["generation"])
        self.assertNotEqual(generation_a_root, generation_b_root)
        self.assertEqual(
            {
                filename: (generation_a_root / filename).read_bytes()
                for filename in MEMBER_NAMES
            },
            generation_a_bytes,
            "publishing generation B must not mutate generation A",
        )
        self.assertEqual(
            {
                path.name
                for path in (self.target / "generations").iterdir()
                if path.is_dir() and not path.name.startswith(".")
            },
            {generation_a_root.name, generation_b_root.name},
        )
        self.assertEqual(
            list(self.target.glob(".current.*"))
            + list((self.target / "generations").glob(".staging.*")),
            [],
            "successful publication must not leak temporary artifacts",
        )

    def test_interrupted_selector_replace_preserves_previous_generation(self) -> None:
        generation_a = self.write_source_generation(b"A")
        self.publish()
        manifest_a_bytes = (self.target / "current.json").read_bytes()
        manifest_a = json.loads(manifest_a_bytes)
        self.assert_selected_generation(manifest_a, generation_a)

        generation_b = self.write_source_generation(b"B")
        failure_marker = self.root / "fail-selector-once"
        self.publish(expected_exit=73, fail_selector_marker=failure_marker)

        self.assertTrue(failure_marker.is_file())
        self.assertEqual(
            (self.target / "current.json").read_bytes(),
            manifest_a_bytes,
            "failure before atomic selector replacement must leave generation A selected",
        )
        self.assertEqual(
            list(self.target.glob(".current.*"))
            + list((self.target / "generations").glob(".staging.*")),
            [],
            "interrupted publication cleanup must remove only temporary artifacts",
        )

        self.publish()
        manifest_b = self.current_manifest()
        self.assert_selected_generation(manifest_b, generation_b)
        self.assertNotEqual(manifest_a["generation"], manifest_b["generation"])

    def test_mixed_letsencrypt_lineage_is_rejected_before_staging(self) -> None:
        self.write_source_generation(b"A")
        self.write_source_generation(b"B")

        completed = self.publish(
            expected_exit=1,
            environment_overrides={"SOROTTE_TEST_CHAIN_LINEAGE": "1"},
        )

        self.assertIn("lineage changed", completed.stderr)
        self.assertFalse(
            self.target.exists(),
            "a mixed source lineage must be rejected before creating publication state",
        )


if __name__ == "__main__":
    unittest.main()
