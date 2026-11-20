#!/usr/bin/env python3
"""Writes the Flatpak sources that vendor every crate in Cargo.lock, for offline builds.

Run from the repository root after changing dependencies:
    python3 packaging/flatpak/cargo-sources.py > packaging/flatpak/cargo-sources.json
"""

import json
import sys
import tomllib

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def main() -> None:
    with open("Cargo.lock", "rb") as file:
        lock = tomllib.load(file)
    sources = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue  # The application itself.
        if source != CRATES_IO:
            sys.exit(f"Unsupported source for {package['name']}: {source}")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    sources.append(
        {
            "type": "inline",
            "contents": (
                "[source.vendored-sources]\n"
                'directory = "cargo/vendor"\n\n'
                "[source.crates-io]\n"
                'replace-with = "vendored-sources"\n'
            ),
            "dest": "cargo",
            "dest-filename": "config.toml",
        }
    )
    json.dump(sources, sys.stdout, indent=4)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
