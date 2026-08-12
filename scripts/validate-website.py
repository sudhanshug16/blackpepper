#!/usr/bin/env python3
"""Validate the static GitHub Pages site without third-party packages."""

from __future__ import annotations

import hashlib
import json
import re
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
HTML_PATH = DOCS / "index.html"
CSS_PATH = DOCS / "assets/site.css"
CANONICAL = "https://sudhanshug16.github.io/blackpepper/"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"website validation failed: {message}")


class SiteParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.ids: set[str] = set()
        self.fragments: list[str] = []
        self.local_assets: list[str] = []
        self.meta: dict[str, str] = {}
        self.links: list[dict[str, str]] = []
        self.tabs: dict[str, dict[str, str]] = {}
        self.panels: dict[str, dict[str, str]] = {}
        self.commands: list[str] = []
        self.product_versions: list[str] = []
        self.lang = ""
        self.charset = ""

    def handle_starttag(self, tag: str, raw_attrs: list[tuple[str, str | None]]) -> None:
        attrs = {key: value or "" for key, value in raw_attrs}
        if tag == "html":
            self.lang = attrs.get("lang", "")
        if tag == "meta":
            self.charset = attrs.get("charset", self.charset)
            key = attrs.get("name") or attrs.get("property")
            if key:
                self.meta[key] = attrs.get("content", "")
        if tag == "link":
            self.links.append(attrs)

        element_id = attrs.get("id")
        if element_id:
            require(element_id not in self.ids, f"duplicate id #{element_id}")
            self.ids.add(element_id)

        href = attrs.get("href")
        if href:
            if href.startswith("#"):
                self.fragments.append(href[1:])
            else:
                self._record_local(href)
        src = attrs.get("src")
        if src:
            self._record_local(src)

        role = attrs.get("role")
        if role == "tab" and element_id:
            self.tabs[element_id] = attrs
        elif role == "tabpanel" and element_id:
            self.panels[element_id] = attrs

        if "data-command" in attrs:
            self.commands.append(attrs["data-command"])
        if "data-product-version" in attrs:
            self.product_versions.append(attrs["data-product-version"])

    def _record_local(self, value: str) -> None:
        parsed = urlsplit(value)
        if not parsed.scheme and not parsed.netloc and parsed.path:
            self.local_assets.append(parsed.path)


def link_with_rel(parser: SiteParser, rel: str) -> list[dict[str, str]]:
    return [link for link in parser.links if rel in link.get("rel", "").split()]


def png_dimensions(path: Path) -> tuple[int, int]:
    with path.open("rb") as image:
        header = image.read(24)
    require(header[:8] == b"\x89PNG\r\n\x1a\n", f"{path.name} is not PNG")
    require(header[12:16] == b"IHDR", f"{path.name} has no IHDR")
    return struct.unpack(">II", header[16:24])


def validate_html(parser: SiteParser) -> None:
    require(parser.lang == "en", "document language must be en")
    require(parser.charset.lower() == "utf-8", "document must declare UTF-8")
    required_ids = {"top", "main", "workflow", "capabilities", "commands", "install"}
    require(required_ids <= parser.ids, "one or more public fragments are missing")
    for fragment in parser.fragments:
        require(fragment in parser.ids, f"fragment target #{fragment} does not exist")

    for relative in parser.local_assets:
        require((DOCS / relative).is_file(), f"local asset {relative} does not exist")

    canonicals = link_with_rel(parser, "canonical")
    require(len(canonicals) == 1, "exactly one canonical link is required")
    require(canonicals[0].get("href") == CANONICAL, "canonical URL is wrong")
    require(link_with_rel(parser, "manifest"), "manifest link is missing")
    require(len(link_with_rel(parser, "icon")) >= 3, "favicon sizes are missing")
    require(link_with_rel(parser, "apple-touch-icon"), "Apple touch icon is missing")

    expected_meta = {
        "description",
        "theme-color",
        "og:type",
        "og:title",
        "og:description",
        "og:url",
        "og:image",
        "og:image:width",
        "og:image:height",
        "og:image:alt",
        "twitter:card",
        "twitter:title",
        "twitter:description",
        "twitter:image",
        "twitter:image:alt",
    }
    require(expected_meta <= parser.meta.keys(), "required social metadata is missing")
    require(parser.meta["og:url"] == CANONICAL, "Open Graph URL differs from canonical")
    require(parser.meta["twitter:card"] == "summary_large_image", "Twitter card type is wrong")

    require(len(parser.tabs) == 4, "command browser must have four tabs")
    selected = [attrs for attrs in parser.tabs.values() if attrs.get("aria-selected") == "true"]
    require(len(selected) == 1, "command browser must start with one selected tab")
    for tab_id, attrs in parser.tabs.items():
        panel_id = attrs.get("aria-controls", "")
        require(panel_id in parser.panels, f"{tab_id} controls a missing panel")
        require(parser.panels[panel_id].get("aria-labelledby") == tab_id, f"{panel_id} is mislabelled")


def validate_parser_backed_commands(parser: SiteParser) -> None:
    source = (ROOT / "crates/blackpepper/src/client/command.rs").read_text()
    help_source = source.split("pub const HELP:", 1)[1].split("];", 1)[0]
    allowed = set(re.findall(r'\(\s*"([^"]+)"\s*,', help_source, re.S))
    require(parser.commands, "website has no parser-backed command examples")
    unknown = sorted(set(parser.commands) - allowed)
    require(not unknown, f"website documents unknown commands: {unknown}")

    cargo = (ROOT / "crates/blackpepper/Cargo.toml").read_text()
    version = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M)
    require(version is not None, "crate version cannot be read")
    require(parser.product_versions == [version.group(1)], "preview version differs from Cargo")


def validate_styles_and_claims() -> None:
    html = HTML_PATH.read_text()
    css = CSS_PATH.read_text()
    combined = f"{html}\n{css}".lower()
    for forbidden in ("backdrop-filter", "gradient(", "box-shadow"):
        require(forbidden not in combined, f"forbidden visual effect {forbidden} found")
    for token in ("#e4834f", "#1c1d1f", "#232427", "#e6e4e1"):
        require(token in css.lower(), f"design token {token} is missing")
    require("position: sticky" in css, "sticky header is missing")
    require("overflow-x: clip" in css, "page overflow guard is missing")
    require("@media (max-width: 620px)" in css, "mobile layout is missing")

    for pattern in (r"main\*", r"PR #[0-9]+", r"tab [0-9]+/[0-9]+", r"[0-9]+m[0-9]+s"):
        require(re.search(pattern, html, re.I) is None, f"unsupported preview data matches {pattern}")
    require("click to forward" in html, "Ports preview lacks truthful mouse control")


def validate_manifest_and_discovery() -> None:
    manifest = json.loads((DOCS / "manifest.webmanifest").read_text())
    require(manifest["name"] == "Blackpepper", "manifest name is wrong")
    require(manifest["start_url"] == "/blackpepper/", "manifest start URL is wrong")
    require(manifest["background_color"] == "#1c1d1f", "manifest background is wrong")
    require(manifest["theme_color"] == "#1c1d1f", "manifest theme color is wrong")
    icons = {icon["sizes"]: icon for icon in manifest["icons"]}
    require({"192x192", "512x512"} <= icons.keys(), "manifest icon sizes are missing")

    robots = (DOCS / "robots.txt").read_text()
    require(f"Sitemap: {CANONICAL}sitemap.xml" in robots, "robots sitemap is wrong")
    sitemap = ET.parse(DOCS / "sitemap.xml")
    loc = sitemap.find("{http://www.sitemaps.org/schemas/sitemap/0.9}url/{http://www.sitemaps.org/schemas/sitemap/0.9}loc")
    require(loc is not None and loc.text == CANONICAL, "sitemap canonical is wrong")


def validate_brand_assets() -> None:
    sizes = {
        "favicon-16.png": (16, 16),
        "favicon-32.png": (32, 32),
        "favicon-48.png": (48, 48),
        "apple-touch-icon.png": (180, 180),
        "app-icon-192.png": (192, 192),
        "app-icon-512.png": (512, 512),
        "social-card.png": (1280, 640),
    }
    asset_dir = DOCS / "assets"
    for name, expected in sizes.items():
        require(png_dimensions(asset_dir / name) == expected, f"{name} dimensions are wrong")

    hash_file = asset_dir / "brand-sha256s.txt"
    recorded = {}
    for line in hash_file.read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        recorded[name] = digest
    require(recorded.keys() == sizes.keys(), "brand hash inventory is incomplete")
    for name, digest in recorded.items():
        actual = hashlib.sha256((asset_dir / name).read_bytes()).hexdigest()
        require(actual == digest, f"{name} differs from deterministic export")

    board = asset_dir / "blackpepper-v2-design-board.webp"
    board_hash = hashlib.sha256(board.read_bytes()).hexdigest()
    require(board_hash == "92009f51af77a88e0b51a19488493df2abb3f92e73c4fd25ad341e6c0636530c", "design board differs from supplied archive")
    license_text = (asset_dir / "fonts/OFL.txt").read_text()
    require("SIL OPEN FONT LICENSE Version 1.1" in license_text, "font license is missing")

    if subprocess.run(["sh", "-c", "command -v magick >/dev/null"], check=False).returncode == 0:
        with tempfile.TemporaryDirectory(prefix="blackpepper-brand-") as output:
            subprocess.run(["bash", str(ROOT / "scripts/generate-brand-assets.sh"), output], check=True, stdout=subprocess.DEVNULL)
            for name in [*sizes, "brand-sha256s.txt"]:
                require((Path(output) / name).read_bytes() == (asset_dir / name).read_bytes(), f"{name} is not reproducible")


def validate_design_captures() -> None:
    asset_dir = DOCS / "assets"
    for name, expected in {
        "site-1280x720.png": (1280, 720),
        "site-390x844.png": (390, 844),
    }.items():
        require(png_dimensions(asset_dir / name) == expected, f"{name} dimensions are wrong")


def main() -> None:
    parser = SiteParser()
    parser.feed(HTML_PATH.read_text())
    parser.close()
    validate_html(parser)
    validate_parser_backed_commands(parser)
    validate_styles_and_claims()
    validate_manifest_and_discovery()
    validate_brand_assets()
    validate_design_captures()
    print(
        "website validation passed "
        f"({len(parser.tabs)} tabs, {len(parser.commands)} parser-backed commands, "
        "7 brand rasters, 2 design captures)"
    )


if __name__ == "__main__":
    main()
