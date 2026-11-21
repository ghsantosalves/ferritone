#!/bin/bash
set -euo pipefail

cd -- "$(dirname -- "$0")/.."
root=$PWD
. /etc/os-release
if [[ ${ID:-} != debian || ${VERSION_CODENAME:-} != trixie ]]; then
    printf '%s\n' 'Build this package on Debian 13 (trixie).' >&2
    exit 1
fi

arch=$(dpkg --print-architecture)
rust_target=$(rustc -vV | sed -n 's/^host: //p')
if [[ "$arch" != amd64 || "$rust_target" != x86_64-unknown-linux-gnu ]]; then
    printf '%s\n' 'This package build currently targets native Debian 13 amd64.' >&2
    exit 1
fi

export CARGO_TARGET_DIR="$root/target"
version=$(cargo metadata --locked --no-deps --format-version=1 | python3 -c     'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
package_version="$version-1~trixie1"

cargo build --release --locked
mkdir -p target/dist
work=$(mktemp -d "$root/target/deb-build.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
package="$work/debian/ferritone"
appid=io.github.ghsantosalves.Ferritone
doc="$package/usr/share/doc/ferritone"

install -Dm755 target/release/ferritone "$package/usr/bin/ferritone"
install -Dm644 "assets/$appid.desktop" "$package/usr/share/applications/$appid.desktop"
install -Dm644 "assets/$appid.metainfo.xml" "$package/usr/share/metainfo/$appid.metainfo.xml"
install -Dm644 assets/player-icon.svg "$package/usr/share/icons/hicolor/scalable/apps/$appid.svg"
install -Dm644 packaging/copyright "$doc/copyright"
cargo metadata --locked --filter-platform "$rust_target" --format-version=1 |
    python3 packaging/collect-crate-licenses.py "$doc/third-party-licenses"

mkdir -p "$package/DEBIAN"
# dpkg-shlibdeps expects a source control file even when writing dependencies to stdout.
cat > "$work/debian/control" <<'CONTROL'
Source: ferritone

Package: ferritone
Architecture: any
CONTROL
depends=$(cd "$work" && env -u LD_LIBRARY_PATH dpkg-shlibdeps -O -e "$package/usr/bin/ferritone")
depends=${depends#shlibs:Depends=}
if [[ -z "$depends" ]]; then
    printf '%s\n' 'Unable to derive shared-library dependencies.' >&2
    exit 1
fi
cat > "$package/DEBIAN/control" <<CONTROL
Package: ferritone
Version: $package_version
Architecture: $arch
Section: sound
Priority: optional
Maintainer: ghsantosalves <40925712+ghsantosalves@users.noreply.github.com>
Installed-Size: $(du -sk "$package/usr" | cut -f1)
Depends: $depends, gstreamer1.0-plugins-base, gstreamer1.0-plugins-good, libglib2.0-bin
Homepage: https://github.com/ghsantosalves/ferritone
Description: Music player for local audio collections
 Ferritone provides a searchable library, drag-and-drop playlists, a
 persistent queue, and a mini player with audio visualizations. It also
 supports optional lyrics, ReplayGain, and desktop media controls.
CONTROL

output="$root/target/dist/ferritone_${package_version}_${arch}.deb"
dpkg-deb --root-owner-group --build "$package" "$output"
