from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import mutation_ci


class MutationTargetIsolationTests(unittest.TestCase):
    def test_parallel_workers_override_all_inherited_shared_build_paths(self):
        inherited = {
            "CARGO_TARGET_DIR": "C:/shared/target",
            "CARGO_BUILD_TARGET_DIR": "C:/other/target",
            "CARGO_BUILD_BUILD_DIR": "C:/shared/intermediate",
            "CARGO_HOME": "C:/registry-cache",
        }
        completed = subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.dict(os.environ, inherited), mock.patch.object(
            mutation_ci.mutation_process, "run", return_value=completed
        ) as run:
            mutation_ci.run_process(["cargo", "mutants", "--jobs", "2"], cwd=pathlib.Path.cwd())
            environment = run.call_args.kwargs["env"]
            for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"):
                self.assertEqual(environment[key], "target")
            self.assertEqual(environment["CARGO_HOME"], inherited["CARGO_HOME"])
            self.assertEqual(os.environ["CARGO_TARGET_DIR"], inherited["CARGO_TARGET_DIR"])

    def test_ordinary_inventory_keeps_its_selected_build_cache(self):
        completed = subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.object(mutation_ci.mutation_process, "run", return_value=completed) as run:
            mutation_ci.run_process(["cargo", "test", "--list"], cwd=pathlib.Path.cwd())
            self.assertIsNone(run.call_args.kwargs["env"])


if __name__ == "__main__":
    unittest.main()
