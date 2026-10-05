#!/usr/bin/env python3
"""Symbolize FreeWorldOS panic backtraces from a QEMU serial log."""

from __future__ import annotations

import argparse
import pathlib
import re
import shutil
import subprocess
import sys

BT_RE = re.compile(r"FreeWorldOS: BT\[(\d+)\]=(0x[0-9a-fA-F]+)")
BASE_RE = re.compile(r"\bvirt_base=(0x[0-9a-fA-F]+)\b")


def choose_addr2line(explicit: str | None) -> str:
    if explicit:
        return explicit
    for candidate in ("llvm-addr2line", "addr2line"):
        found = shutil.which(candidate)
        if found:
            return found
    raise RuntimeError("llvm-addr2line/addr2line not found")


def parse_log(text: str) -> tuple[int, list[tuple[int, int]]]:
    base_match = BASE_RE.search(text)
    base = int(base_match.group(1), 16) if base_match else 0

    frames = [
        (int(index), int(address, 16))
        for index, address in BT_RE.findall(text)
    ]
    return base, frames


def symbolize(tool: str, kernel: pathlib.Path, address: int) -> str:
    result = subprocess.run(
        [tool, "-f", "-C", "-e", str(kernel), hex(address)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--log", required=True, type=pathlib.Path)
    parser.add_argument(
        "--base",
        type=lambda value: int(value, 0),
        help="override kernel virtual load base; default is parsed from serial log",
    )
    parser.add_argument("--addr2line", help="addr2line-compatible tool")
    parser.add_argument(
        "--no-rebase",
        action="store_true",
        help="pass runtime addresses directly to addr2line",
    )
    args = parser.parse_args()

    try:
        text = args.log.read_text(encoding="utf-8", errors="replace")
        parsed_base, frames = parse_log(text)
        tool = choose_addr2line(args.addr2line)
    except (OSError, RuntimeError) as error:
        print(error, file=sys.stderr)
        return 2

    if not frames:
        print("no FreeWorldOS BT[...] frames found", file=sys.stderr)
        return 1

    base = args.base if args.base is not None else parsed_base

    for index, runtime in frames:
        lookup = runtime if args.no_rebase else runtime - base
        if lookup < 0:
            print(
                f"BT[{index:02}] runtime={runtime:#x}: address below base {base:#x}",
                file=sys.stderr,
            )
            continue

        decoded = symbolize(tool, args.kernel, lookup)
        print(
            f"BT[{index:02}] runtime={runtime:#018x} "
            f"lookup={lookup:#x}\n{decoded}"
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
