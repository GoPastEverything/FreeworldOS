#!/usr/bin/env python3
"""Decode FreeWorldOS event IDs from the shared kernel/events.def table."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

EVENT_RE = re.compile(
    r"^\s*fw_event!\(\s*([A-Z0-9_]+)\s*,\s*(0x[0-9a-fA-F]+|[0-9]+)\s*\);\s*$"
)
SERIAL_EVENT_RE = re.compile(r"\bid=(0x[0-9a-fA-F]+|[0-9]+)\b")


def load_events(path: pathlib.Path) -> dict[int, str]:
    events: dict[int, str] = {}
    names: set[str] = set()

    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("//"):
            continue

        match = EVENT_RE.match(line)
        if not match:
            raise ValueError(f"{path}:{number}: invalid event definition: {line!r}")

        name, raw_id = match.groups()
        event_id = int(raw_id, 0)

        if name in names:
            raise ValueError(f"{path}:{number}: duplicate event name {name}")
        if event_id in events:
            raise ValueError(
                f"{path}:{number}: duplicate event id {event_id:#x} "
                f"({events[event_id]} and {name})"
            )

        names.add(name)
        events[event_id] = name

    if not events:
        raise ValueError(f"{path}: no events found")

    return events


def render_stream(events: dict[int, str], stream) -> None:
    for line in stream:
        match = SERIAL_EVENT_RE.search(line)
        if not match:
            print(line, end="")
            continue

        event_id = int(match.group(1), 0)
        name = events.get(event_id, "UNKNOWN_EVENT")
        print(line.rstrip("\n") + f" event_name={name}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--table",
        type=pathlib.Path,
        default=pathlib.Path("kernel/events.def"),
        help="shared FreeWorld event table",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate the event table and print a summary",
    )
    parser.add_argument(
        "--decode",
        type=lambda value: int(value, 0),
        help="decode one numeric event ID",
    )
    parser.add_argument(
        "--log",
        type=pathlib.Path,
        help="annotate EVENT lines from a serial log; stdin if omitted",
    )
    args = parser.parse_args()

    try:
        events = load_events(args.table)
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 2

    if args.check:
        print(f"FreeWorldOS events: {len(events)} unique IDs")
        return 0

    if args.decode is not None:
        print(events.get(args.decode, "UNKNOWN_EVENT"))
        return 0

    if args.log:
        with args.log.open("r", encoding="utf-8", errors="replace") as stream:
            render_stream(events, stream)
    else:
        render_stream(events, sys.stdin)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
