#!/usr/bin/env python3
"""Read the current acceptance runtime from Blackpepper's Rust pin/manifest."""

from __future__ import annotations

import argparse
import hashlib
import re
import tarfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VERSION_SOURCE = REPOSITORY_ROOT / "crates/blackpepper/src/transport/sidecar.rs"
MANIFEST_SOURCE = (
    REPOSITORY_ROOT / "crates/blackpepper/src/transport/sidecar_manifest.rs"
)
VERSION_PATTERN = re.compile(
    r'^pub const ZELLIJ_VERSION: &str = "([^"]+)";$', re.MULTILINE
)
VALID_VERSION = re.compile(r"[0-9][0-9A-Za-z.+_-]{0,63}")
ASSET_BLOCK_PATTERN = re.compile(
    r"^\s*ReleaseAsset \{\n(.*?)^\s*\},$", re.MULTILINE | re.DOTALL
)
TARGETS = {
    "x86_64-unknown-linux-musl": "LinuxX86_64",
}


class MetadataError(RuntimeError):
    """The Rust pin or manifest cannot be interpreted unambiguously."""


@dataclass(frozen=True)
class ZellijAsset:
    version: str
    target: str
    asset_name: str
    binary_name: str
    archive_sha256: str
    binary_sha256: str | None

    @property
    def cache_relative(self) -> str:
        return f"blackpepper/sidecars/zellij/{self.version}/{self.target}"


def current_zellij_version() -> str:
    matches = VERSION_PATTERN.findall(VERSION_SOURCE.read_text())
    if len(matches) != 1 or VALID_VERSION.fullmatch(matches[0]) is None:
        raise MetadataError(
            f"expected one valid ZELLIJ_VERSION declaration in {VERSION_SOURCE}"
        )
    return matches[0]


def _field(block: str, name: str) -> str:
    matches = re.findall(rf'^\s*{name}: "([^"]+)",$', block, re.MULTILINE)
    if len(matches) != 1:
        raise MetadataError(f"asset has no unambiguous {name} field")
    return matches[0]


def _enum_field(block: str, name: str, enum_name: str) -> str:
    matches = re.findall(
        rf"^\s*{name}: {enum_name}::([A-Za-z0-9_]+),$", block, re.MULTILINE
    )
    if len(matches) != 1:
        raise MetadataError(f"asset has no unambiguous {name} field")
    return matches[0]


def _digest_field(block: str, name: str) -> str | None:
    none_pattern = re.compile(rf"^\s*{name}: None,$", re.MULTILINE)
    some_pattern = re.compile(
        rf'^\s*{name}: Some\(\s*"([0-9a-fA-F]{{64}})",?\s*\),',
        re.MULTILINE | re.DOTALL,
    )
    is_none = none_pattern.search(block) is not None
    matches = some_pattern.findall(block)
    if is_none == (len(matches) == 1):
        raise MetadataError(f"asset has no unambiguous {name} field")
    return None if is_none else matches[0].lower()


def current_zellij_asset(target: str) -> ZellijAsset:
    target_variant = TARGETS.get(target)
    if target_variant is None:
        raise MetadataError(f"unsupported acceptance target: {target}")
    version = current_zellij_version()
    matches: list[ZellijAsset] = []
    for block in ASSET_BLOCK_PATTERN.findall(MANIFEST_SOURCE.read_text()):
        if _enum_field(block, "tool", "ManagedTool") != "Zellij":
            continue
        if _enum_field(block, "target", "SidecarTarget") != target_variant:
            continue
        if _field(block, "version") != version:
            continue
        archive_sha256 = _digest_field(block, "trusted_sha256")
        if archive_sha256 is None:
            raise MetadataError("current Zellij asset has no trusted archive checksum")
        matches.append(
            ZellijAsset(
                version=version,
                target=target,
                asset_name=_field(block, "asset_name"),
                binary_name=_field(block, "binary_name"),
                archive_sha256=archive_sha256,
                binary_sha256=_digest_field(block, "binary_sha256"),
            )
        )
    if len(matches) != 1:
        raise MetadataError(
            f"expected one current Zellij asset for {target} in {MANIFEST_SOURCE}"
        )
    return matches[0]


def archive_binary_sha256(asset: ZellijAsset, archive_path: Path) -> str:
    archive_digest = hashlib.sha256()
    with archive_path.open("rb") as archive_file:
        for chunk in iter(lambda: archive_file.read(1024 * 1024), b""):
            archive_digest.update(chunk)
    if archive_digest.hexdigest() != asset.archive_sha256:
        raise MetadataError(
            f"archive checksum does not match the current manifest: {archive_path}"
        )

    try:
        with tarfile.open(archive_path, "r:*") as archive:
            members = [
                member
                for member in archive.getmembers()
                if member.isfile()
                and not PurePosixPath(member.name).is_absolute()
                and ".." not in PurePosixPath(member.name).parts
                and PurePosixPath(member.name).name == asset.binary_name
            ]
            if len(members) != 1:
                raise MetadataError(
                    f"expected one {asset.binary_name} file in {archive_path}"
                )
            extracted = archive.extractfile(members[0])
            if extracted is None:
                raise MetadataError(
                    f"could not read {asset.binary_name} from {archive_path}"
                )
            binary_digest = hashlib.sha256()
            for chunk in iter(lambda: extracted.read(1024 * 1024), b""):
                binary_digest.update(chunk)
            return binary_digest.hexdigest()
    except tarfile.TarError as error:
        raise MetadataError(f"could not read Zellij archive {archive_path}: {error}") from error


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("version")
    asset = subcommands.add_parser("asset")
    asset.add_argument("target", choices=sorted(TARGETS))
    archive_binary = subcommands.add_parser("archive-binary-sha256")
    archive_binary.add_argument("target", choices=sorted(TARGETS))
    archive_binary.add_argument("archive", type=Path)
    return parser.parse_args()


def main() -> int:
    args = arguments()
    if args.command == "version":
        print(current_zellij_version())
        return 0
    asset = current_zellij_asset(args.target)
    if args.command == "archive-binary-sha256":
        print(archive_binary_sha256(asset, args.archive))
        return 0
    print(asset.version)
    print(asset.cache_relative)
    print(asset.asset_name)
    print(asset.binary_sha256 or "-")
    print(asset.binary_name)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (MetadataError, OSError) as error:
        raise SystemExit(f"Zellij acceptance metadata error: {error}") from error
