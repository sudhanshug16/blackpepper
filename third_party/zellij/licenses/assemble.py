#!/usr/bin/env python3
"""Append pinned Rust runtime notices to cargo-about's HTML output."""

import argparse
import html
from pathlib import Path


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def rust_library_body(document: str) -> str:
    opening = "<body>"
    closing = "</body>"
    if document.count(opening) != 1 or document.count(closing) != 1:
        raise ValueError("Rust library copyright file has unexpected HTML structure")
    return document.split(opening, 1)[1].split(closing, 1)[0].strip()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--licenses", type=Path, required=True)
    parser.add_argument("--rust-library", type=Path, required=True)
    parser.add_argument("--musl", type=Path, required=True)
    parser.add_argument("--libunwind", type=Path, required=True)
    args = parser.parse_args()

    licenses = read(args.licenses)
    insertion_point = "    </main>"
    if licenses.count(insertion_point) != 1:
        raise ValueError("cargo-about output has unexpected HTML structure")

    supplement = f"""
        <section id="rust-runtime-notices">
            <h2>Rust standard library and toolchain runtime notices</h2>
            <p>
                These notices cover code from the pinned Rust standard library
                and the musl/libunwind runtimes linked into static Linux builds.
            </p>
            {rust_library_body(read(args.rust_library))}
            <h3>musl libc</h3>
            <pre>{html.escape(read(args.musl))}</pre>
            <h3>LLVM libunwind</h3>
            <pre>{html.escape(read(args.libunwind))}</pre>
        </section>
"""
    args.licenses.write_text(
        licenses.replace(insertion_point, supplement + insertion_point),
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
