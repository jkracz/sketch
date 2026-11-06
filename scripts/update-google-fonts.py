#!/usr/bin/env python3
"""Regenerates assets/fonts/google-fonts.json, the list of Google Fonts families.

Run it with GOOGLE_FONTS_API_KEY set. Only this script uses the key, and the file it writes holds
names and styles, not fonts.
"""
import json
import os
import sys
import urllib.request
from pathlib import Path

URL = "https://www.googleapis.com/webfonts/v1/webfonts?sort=popularity"
OUT = Path(__file__).resolve().parent.parent / "assets/fonts/google-fonts.json"


def weights(variants, italic):
    """The numeric weights among variants like "regular", "italic", "700" and "700italic"."""
    out = []
    for v in variants:
        name = {"regular": "400", "italic": "400italic"}.get(v, v)
        if name.endswith("italic") == italic:
            out.append(int(name.removesuffix("italic")))
    return sorted(out)


def main():
    key = os.environ.get("GOOGLE_FONTS_API_KEY")
    if not key:
        sys.exit("Set GOOGLE_FONTS_API_KEY")
    request = urllib.request.Request(URL, headers={"X-Goog-Api-Key": key})
    with urllib.request.urlopen(request) as response:
        items = json.load(response)["items"]
    # Text renders as one-color outlines, so color fonts would lose what makes them worth using.
    families = [
        {
            "family": f["family"],
            "category": f["category"],
            "weights": weights(f["variants"], False),
            "italics": weights(f["variants"], True),
        }
        for f in items
        if not f.get("colorCapabilities")
    ]
    # One family per line, in popularity order, so updates read as small diffs.
    lines = ",\n".join(json.dumps(f, separators=(",", ":")) for f in families)
    OUT.write_text(f"[\n{lines}\n]\n")
    print(f"{len(families)} of {len(items)} families written to {OUT}")


if __name__ == "__main__":
    main()
