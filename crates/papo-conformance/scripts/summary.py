#!/usr/bin/env python3
"""Summarize papo-conformance libtest JSON from standard input."""

import json
import sys


def summarize(lines):
    """Return scenario statuses from libtest and papo scenario records."""
    return summarize_with_evidence(lines)[0]


def summarize_with_evidence(lines):
    """Return scenario statuses and their recorded wire evidence."""
    statuses = {}
    evidence = {}
    for line in lines:
        record = json.loads(line)
        name = record.get("name")
        event = record.get("event")
        if record.get("type") == "evidence":
            evidence[name] = record["value"]
        elif record.get("type") == "scenario":
            if event == "ignored" and record.get("reason"):
                event = f"ignored ({record['reason']})"
            statuses[name] = event
        elif record.get("type") == "test" and event in {
            "ok",
            "failed",
            "ignored",
        }:
            status = "passed" if event == "ok" else event
            statuses.setdefault(name, status)
    return statuses, evidence


def render(statuses):
    """Render statuses in stable scenario-name order."""
    return "".join(f"{name} -> {statuses[name]}\n" for name in sorted(statuses))


if __name__ == "__main__":
    sys.stdout.write(render(summarize(sys.stdin)))
