#!/usr/bin/env python3
import pathlib
import runpy
import unittest


PATH = pathlib.Path(__file__).with_name("conformance-matrix")
MODULE = runpy.run_path(PATH)


class MatrixTests(unittest.TestCase):
    def test_renders_every_outcome_and_known_failure_reason(self):
        MODULE["render"].__globals__["PROFILES"] = ("ergo-default", "soju-ergo")
        MODULE["render"].__globals__["PROFILE_LABELS"] = {
            "ergo-default": "Ergo default",
            "soju-ergo": "soju + Ergo",
        }
        MODULE["render"].__globals__["KNOWN_FAILURES"] = {
            ("soju-ergo", "known"): {
                "kind": "not-implemented",
                "reason": "wire difference",
            }
        }
        output = MODULE["render"](
            {
                "ergo-default": {
                    "fail": "failed",
                    "known": "passed",
                    "skip": "ignored (cap)",
                },
                "soju-ergo": {
                    "fail": "passed",
                    "known": "expected-failure",
                    "skip": "passed",
                },
            }
        )
        self.assertIn("| `fail` | ✗ | ✓ |", output)
        self.assertIn("| `known` | ✓ | ⚠ [^1] |", output)
        self.assertIn("| `skip` | — | ✓ |", output)
        self.assertIn(
            "[^1]: ⚠ not implemented: `soju-ergo` / `known` — wire difference",
            output,
        )
