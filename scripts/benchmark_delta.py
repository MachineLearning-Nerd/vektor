#!/usr/bin/env python3
"""Render an informational retrieval benchmark delta summary."""

from __future__ import annotations

import re
import sys
from pathlib import Path


METRICS = ("Precision@5", "Recall@5", "MRR")


def parse_metrics(text: str) -> dict[str, float]:
    values: dict[str, float] = {}
    for metric in METRICS:
        match = re.search(rf"{re.escape(metric)}:\s*([0-9]+(?:\.[0-9]+)?)", text)
        if not match:
            match = re.search(
                rf"^\|\s*{re.escape(metric)}\s*\|\s*([0-9]+(?:\.[0-9]+)?)\s*\|",
                text,
                re.MULTILINE,
            )
        if match:
            values[metric] = float(match.group(1))
    missing = [metric for metric in METRICS if metric not in values]
    if missing:
        raise ValueError(f"missing benchmark metrics: {', '.join(missing)}")
    return values


def parse_query_count(text: str) -> int | None:
    match = re.search(r"Queries:\s*(\d+)", text)
    return int(match.group(1)) if match else None


def render_summary(current: dict[str, float], baseline: dict[str, float], queries: int | None) -> str:
    lines = [
        "<!-- vektor-benchmark-delta -->",
        "## Vektor retrieval benchmark",
        "",
    ]
    if queries is not None:
        lines.append(f"- Queries: {queries}")
    lines.extend(
        [
            "- Mode: informational only for `v0.4.0`",
            "",
            "| Metric | Baseline | Current | Delta |",
            "|---|---:|---:|---:|",
        ]
    )
    for metric in METRICS:
        delta = current[metric] - baseline[metric]
        lines.append(f"| {metric} | {baseline[metric]:.3f} | {current[metric]:.3f} | {delta:+.3f} |")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    if len(sys.argv) != 4:
        print(
            "usage: benchmark_delta.py <benchmark-output> <baseline-md> <summary-out>",
            file=sys.stderr,
        )
        return 2

    output_path = Path(sys.argv[1])
    baseline_path = Path(sys.argv[2])
    summary_path = Path(sys.argv[3])

    current_text = output_path.read_text(encoding="utf-8")
    baseline_text = baseline_path.read_text(encoding="utf-8")
    current = parse_metrics(current_text)
    baseline = parse_metrics(baseline_text)
    queries = parse_query_count(current_text)
    summary_path.write_text(render_summary(current, baseline, queries), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
