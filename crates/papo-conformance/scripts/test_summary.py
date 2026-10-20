#!/usr/bin/env python3
"""Tests for the conformance JSON summarizer."""

import io
import unittest

from summary import render, summarize, summarize_with_evidence


SAMPLE = """\
{ "type": "suite", "event": "started", "test_count": 3 }
{ "type": "test", "event": "started", "name": "registration" }
{ "type": "evidence", "name": "registration", "value": "longest S: 487 bytes, CAP" }
{ "type": "test", "name": "registration", "event": "ok" }
{ "type": "test", "event": "started", "name": "websocket" }
{ "type": "scenario", "name": "websocket", "event": "ignored", "reason": "PAPO_TEST_WS is not set up" }
{ "type": "test", "name": "websocket", "event": "ignored" }
{ "type": "test", "event": "started", "name": "metadata" }
{ "type": "scenario", "name": "metadata", "event": "expected-failure", "reason": "server does not implement metadata" }
{ "type": "test", "name": "metadata", "event": "ok" }
{ "type": "suite", "event": "ok", "passed": 2, "failed": 0, "ignored": 1 }
"""


class SummaryTests(unittest.TestCase):
    def test_captured_libtest_json_preserves_scenario_outcomes(self):
        output = render(summarize(io.StringIO(SAMPLE)))
        self.assertEqual(
            output,
            "metadata -> expected-failure\n"
            "registration -> passed\n"
            "websocket -> ignored (PAPO_TEST_WS is not set up)\n",
        )

    def test_empty_input_has_no_scenarios(self):
        self.assertEqual(summarize(io.StringIO("")), {})

    def test_keeps_scenario_evidence_separate_from_status(self):
        statuses, evidence = summarize_with_evidence(io.StringIO(SAMPLE))
        self.assertEqual(statuses["registration"], "passed")
        self.assertEqual(evidence["registration"], "longest S: 487 bytes, CAP")


if __name__ == "__main__":
    unittest.main()
