from __future__ import annotations

import importlib.util
import pathlib
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
HARNESS_PATH = REPO_ROOT / "scripts" / "persistence_power_loss_harness.py"
RUST_DRIVER_PATH = (
    REPO_ROOT
    / "crates"
    / "sorotte-server"
    / "src"
    / "tests"
    / "persistence_power_loss_harness_tests.rs"
)


def load_harness() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location(
        "persistence_power_loss_harness", HARNESS_PATH
    )
    if spec is None or spec.loader is None:
        raise AssertionError("power-loss harness module must be loadable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class PersistencePowerLossHarnessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.harness = load_harness()

    def test_plan_is_fixed_to_new_owned_images(self) -> None:
        plan = self.harness.plan_document(REPO_ROOT)
        self.assertEqual(plan["schema"], "sorotte-disposable-powerloss-v1")
        self.assertEqual(plan["mode"], "read-only-plan")
        self.assertFalse(plan["network_activity"])
        self.assertEqual(plan["accepted_existing_device_or_mount_arguments"], [])
        self.assertEqual(plan["workspace_parent"], "/var/tmp")
        self.assertEqual(
            plan["image_sizes_bytes"],
            {
                "live-data.img": 256 * 1024 * 1024,
                "write-log.img": 512 * 1024 * 1024,
                "replay-baseline.img": 256 * 1024 * 1024,
                "replay-app-ack.img": 256 * 1024 * 1024,
                "replay-syncfs.img": 256 * 1024 * 1024,
            },
        )
        self.assertIn("dm-log-writes", plan["block_stack"])
        self.assertFalse(plan["automatic_cleanup"])
        self.assertIn("not durability evidence", plan["claim_limit"])

    def test_cli_exposes_no_device_image_or_mount_target_option(self) -> None:
        plan_args = self.harness.parse_args(["--plan-json"])
        self.assertTrue(plan_args.plan_json)
        run_args = self.harness.parse_args(
            [
                "--run",
                "--confirm",
                self.harness.CONFIRMATION_TOKEN,
            ]
        )
        self.assertTrue(run_args.run)
        self.assertEqual(
            vars(run_args).keys(),
            {"plan_json", "preflight", "run", "confirm"},
        )

    def test_nonprivileged_preflight_is_read_only_and_fail_closed(self) -> None:
        with (
            mock.patch.object(self.harness.shutil, "which", return_value=None),
            mock.patch.object(self.harness.platform, "system", return_value="Windows"),
            mock.patch.object(self.harness.platform, "release", return_value="test"),
        ):
            preflight = self.harness.collect_preflight(REPO_ROOT)
        self.assertEqual(preflight["mode"], "read-only-preflight")
        self.assertFalse(preflight["destructive_actions_attempted"])
        self.assertFalse(preflight["capability_prerequisites_present"])
        self.assertFalse(preflight["ready_for_privileged_run"])
        self.assertEqual(
            preflight["missing_tools"],
            sorted(self.harness.REQUIRED_TOOLS),
        )

    def test_owned_image_guards_reject_outside_wrong_size_and_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as parent_text:
            parent = pathlib.Path(parent_text)
            workspace = self.harness.create_owned_workspace(parent)
            image = self.harness.create_sparse_image(
                workspace,
                "live-data.img",
                self.harness.DATA_IMAGE_BYTES,
            )
            self.assertEqual(
                self.harness.assert_owned_image(
                    workspace, image, self.harness.DATA_IMAGE_BYTES
                ),
                image.resolve(strict=True),
            )

            outside = parent / "outside.img"
            outside.write_bytes(b"")
            with self.assertRaisesRegex(
                self.harness.SafetyError, "fixed owned image specification"
            ):
                self.harness.assert_owned_image(
                    workspace, outside, self.harness.DATA_IMAGE_BYTES
                )

            with image.open("r+b") as handle:
                handle.truncate(self.harness.DATA_IMAGE_BYTES - 1)
            with self.assertRaisesRegex(self.harness.SafetyError, "exactly"):
                self.harness.assert_owned_image(
                    workspace, image, self.harness.DATA_IMAGE_BYTES
                )
            with image.open("r+b") as handle:
                handle.truncate(self.harness.DATA_IMAGE_BYTES)

            link = workspace.root / "replay-baseline.img"
            try:
                link.symlink_to(image)
            except OSError:
                pass
            else:
                with self.assertRaisesRegex(
                    self.harness.SafetyError, "regular file"
                ):
                    self.harness.assert_owned_image(
                        workspace,
                        link,
                        self.harness.DATA_IMAGE_BYTES,
                    )

            for candidate in workspace.root.iterdir():
                if candidate.is_file() or candidate.is_symlink():
                    try:
                        candidate.chmod(0o600)
                    except OSError:
                        pass
            shutil.rmtree(workspace.root)

    def test_owned_root_guard_rejects_mismatched_marker(self) -> None:
        with tempfile.TemporaryDirectory() as parent_text:
            parent = pathlib.Path(parent_text)
            workspace = self.harness.create_owned_workspace(parent)
            workspace.marker.chmod(0o600)
            workspace.marker.write_text(
                self.harness._marker_contents("f" * 32),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(self.harness.SafetyError, "does not match"):
                self.harness.assert_owned_root(workspace)
            shutil.rmtree(workspace.root)

    def test_recorder_uses_argument_arrays_without_a_shell(self) -> None:
        recorder = self.harness.Recorder()
        completed = subprocess.CompletedProcess(
            args=["tool", "argument"],
            returncode=0,
            stdout="ok\n",
            stderr="",
        )
        with mock.patch.object(
            self.harness.subprocess, "run", return_value=completed
        ) as run:
            result = recorder.run(["tool", "argument"])
        self.assertEqual(result.stdout, "ok\n")
        _, kwargs = run.call_args
        self.assertNotIn("shell", kwargs)
        self.assertEqual(recorder.commands[0]["argv"], ["tool", "argument"])

    def test_dm_log_writes_table_requires_exact_ordered_loop_roles(self) -> None:
        self.harness.validate_log_writes_table(
            "0 524288 log-writes 7:3 7:9",
            expected_sectors=524288,
            data_identity=(7, 3),
            log_identity=(7, 9),
        )
        with self.assertRaisesRegex(self.harness.SafetyError, "data operand"):
            self.harness.validate_log_writes_table(
                "0 524288 log-writes 7:9 7:3",
                expected_sectors=524288,
                data_identity=(7, 3),
                log_identity=(7, 9),
            )
        with self.assertRaisesRegex(self.harness.SafetyError, "log operand"):
            self.harness.validate_log_writes_table(
                "0 524288 log-writes 7:3 7:10",
                expected_sectors=524288,
                data_identity=(7, 3),
                log_identity=(7, 9),
            )
        with self.assertRaisesRegex(self.harness.SafetyError, "exactly five"):
            self.harness.validate_log_writes_table(
                "0 524288 log-writes 7:3 7:9 unexpected",
                expected_sectors=524288,
                data_identity=(7, 3),
                log_identity=(7, 9),
            )

    def test_privileged_mode_refuses_missing_confirmation_before_actions(self) -> None:
        with self.assertRaisesRegex(self.harness.SafetyError, "--confirm"):
            self.harness.run_privileged(REPO_ROOT, None)

    def test_source_contract_revalidates_every_destructive_layer(self) -> None:
        source = HARNESS_PATH.read_text(encoding="utf-8")
        for required in (
            "assert_owned_image",
            "assert_owned_loop",
            "assert_owned_mapper",
            "assert_owned_mount",
            "--nooverlap",
            "--direct-io=on",
            "dmsetup",
            "log-writes",
            "validate_log_writes_table",
            "replay-log",
            "--end-mark",
            "replacement-app-ack",
            "replacement-syncfs",
        ):
            self.assertIn(required, source)
        for forbidden in (
            "shell=True",
            "/dev/sd",
            "/dev/nvme",
            "/dev/vd",
            "shutil.rmtree(",
            "os.system(",
        ):
            self.assertNotIn(forbidden, source)

    def test_rust_driver_is_inert_and_checks_exact_owned_path(self) -> None:
        source = RUST_DRIVER_PATH.read_text(encoding="utf-8")
        self.assertIn(
            'if std::env::var_os(ENABLE_ENV).as_deref() '
            '!= Some(std::ffi::OsStr::new(ENABLE_TOKEN))',
            source,
        )
        self.assertIn('let mount = root.join("mount");', source)
        self.assertIn('join("rooms.sqlite3")', source)
        self.assertIn("ownership marker does not match", source)
        self.assertIn('"verify-old-or-new"', source)
        self.assertIn('assert_eq!(integrity, "ok"', source)
        for forbidden in ("TcpStream", "UdpSocket", "std::net", "reqwest"):
            self.assertNotIn(forbidden, source)


if __name__ == "__main__":
    unittest.main()
