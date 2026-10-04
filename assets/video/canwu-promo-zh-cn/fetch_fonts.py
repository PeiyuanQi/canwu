#!/usr/bin/env python3
"""Download the open-licensed (SIL OFL) fonts used by scenes.html into ./fonts.

Noto Serif SC, Noto Sans SC, Ma Shan Zheng, and JetBrains Mono are fetched as
full TrueType files from Google Fonts. The files are large and reproducible,
so they are ignored by git.
"""

import pathlib
import re
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
FONTS = HERE / "fonts"

# (css2 family query, weight -> local file stem)
FAMILIES = [
    ("Noto+Serif+SC:wght@400;600;900", "NotoSerifSC"),
    ("Noto+Sans+SC:wght@400;500;700", "NotoSansSC"),
    ("Ma+Shan+Zheng", "MaShanZheng"),
    ("JetBrains+Mono:wght@400;600", "JetBrainsMono"),
]


def fetch(url: str, user_agent: str = "Wget/1.21") -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": user_agent})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def main() -> None:
    FONTS.mkdir(exist_ok=True)
    for query, stem in FAMILIES:
        # A non-browser user agent makes the CSS API return full .ttf files.
        css = fetch(f"https://fonts.googleapis.com/css2?family={query}").decode("utf-8")
        blocks = re.findall(r"font-weight:\s*(\d+);.*?src:\s*url\(([^)]+)\)", css, re.S)
        for weight, url in blocks:
            name = f"{stem}.ttf" if stem == "MaShanZheng" else f"{stem}-{weight}.ttf"
            target = FONTS / name
            if target.exists() and target.stat().st_size > 0:
                print(f"have {name}")
                continue
            print(f"fetch {name}")
            target.write_bytes(fetch(url))


if __name__ == "__main__":
    main()
