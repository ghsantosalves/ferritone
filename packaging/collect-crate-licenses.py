"""Copy license notices for the crates resolved for the package's build target."""

import json
from pathlib import Path
import sys


COMMON_LICENSES = Path("/usr/share/common-licenses")


def common_license_fallback(package):
    """For crates without bundled notices, pick a license option whose text Debian ships."""
    options = {part.strip() for part in (package["license"] or "").replace("/", " OR ").split(" OR ")}
    for spdx in ("Apache-2.0", "MPL-2.0", "BSL-1.0"):
        if spdx in options and (COMMON_LICENSES / spdx).is_file():
            return spdx, COMMON_LICENSES / spdx
    return None


def main():
    destination = Path(sys.argv[1])
    metadata = json.load(sys.stdin)
    workspace = set(metadata["workspace_members"])
    entries = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["id"] in workspace:
            continue
        name = f'{package["name"]}-{package["version"]}'
        crate = Path(package["manifest_path"]).parent
        notices = {
            path
            for path in crate.iterdir()
            if path.is_file() and path.name.upper().startswith(
                ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE")
            )
        }
        if package["license_file"]:
            notices.add(crate / package["license_file"])
        output = destination / name
        output.mkdir(parents=True)
        if not notices:
            fallback = common_license_fallback(package)
            if fallback is None:
                raise SystemExit(f"No license notice found in {name}")
            spdx, text = fallback
            authors = ", ".join(package["authors"]) or "the upstream authors"
            (output / "1-NOTICE").write_text(
                f"{name} is published as {package['license']} by {authors}.\n"
                f"Upstream: {package['repository'] or 'n/a'}\n"
                f"The crate ships without a license file; Ferritone uses it under {spdx},\n"
                f"whose full text follows in 2-{spdx}.\n",
                encoding="utf-8",
            )
            (output / f"2-{spdx}").write_bytes(text.read_bytes())
        for index, notice in enumerate(sorted(notices), start=1):
            (output / f"{index}-{notice.name}").write_bytes(notice.read_bytes())
        entries.append(f'{name}: {package["license"] or package["license_file"]}')
    (destination / "index.txt").write_text("\n".join(entries) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
