#!/usr/bin/env python3
"""Inline the PNG diagrams into the design compendium as base64 data URIs.

Each <img> in the compendium keeps a data-src attribute naming its source file
under this directory. That attribute is what makes this script idempotent: it
rewrites the src value in place, whether it currently holds a relative path or a
previously embedded data URI.

Usage:  python3 docs/architecture/assets/embed-diagrams.py
"""

import base64
import pathlib
import re
import sys

ASSETS = pathlib.Path(__file__).resolve().parent
DOC = ASSETS.parent / "hyper-cdc-orchestrator-design.html"

IMG = re.compile(r'(<img\s+data-src="([^"]+)"\s+src=")([^"]*)(")')


def main() -> int:
    html = DOC.read_text(encoding="utf-8")
    embedded = []

    def replace(match: re.Match) -> str:
        source = ASSETS.parent / match.group(2)
        if not source.is_file():
            # Older compendium assets may only survive as an embedded data URI.
            # Keep those images intact while allowing newly generated assets to
            # be embedded independently.
            if match.group(3).startswith("data:image/"):
                return match.group(0)
            raise SystemExit(f"missing diagram source: {source}")
        payload = base64.b64encode(source.read_bytes()).decode("ascii")
        embedded.append((match.group(2), source.stat().st_size))
        return f"{match.group(1)}data:image/png;base64,{payload}{match.group(4)}"

    updated = IMG.sub(replace, html)
    if not embedded:
        raise SystemExit("no data-src images found; nothing to embed")

    DOC.write_text(updated, encoding="utf-8")

    for name, size in embedded:
        print(f"embedded {name} ({size:,} bytes)")
    print(f"{DOC.name} is now {DOC.stat().st_size:,} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
