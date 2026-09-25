#!/usr/bin/env bash
# CrystalSKK の MSI を作る (ADR-0037)。
#
#   installer/build.sh [x64|arm64]
#
# 64 ビット (x64 か ARM64) の TIP・辞書サーバ・crystalskk-setup と、32 ビットの
# TIP をビルドし、WiX で MSI にまとめる。出来上がりは target/installer/ に置く。
# WiX は .NET のローカルツールとして入れてある (.config/dotnet-tools.json)。
set -euo pipefail

arch="${1:-x64}"
case "$arch" in
  x64) target=x86_64-pc-windows-msvc ;;
  arm64) target=aarch64-pc-windows-msvc ;;
  *) echo "使い方: $0 [x64|arm64]" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
version="$(cargo metadata --format-version 1 --no-deps \
  | sed -n 's/.*"name":"crystalskk-setup","version":"\([^"]*\)".*/\1/p')"
if [ -z "$version" ]; then
  echo "版が読めません" >&2
  exit 1
fi

cargo build --release --target "$target" \
  -p crystalskk-tip -p crystalskk-server -p crystalskk-setup
cargo build --release --target i686-pc-windows-msvc -p crystalskk-tip

out=target/installer
mkdir -p "$out"
# アプリの一覧に出す絵。TIP と同じく SVG から描く (ADR-0024)。
cargo run --release -p crystalskk-art --example ico -- "$out/face.ico" assets/icons/face.svg
dotnet tool restore
dotnet wix build installer/crystalskk.wxs \
  -arch "$arch" \
  -d Version="$version" \
  -d BinDir="target/$target/release" \
  -d X86Dir="target/i686-pc-windows-msvc/release" \
  -d Icon="$out/face.ico" \
  -o "$out/CrystalSKK-$version-$arch.msi"
echo "$out/CrystalSKK-$version-$arch.msi"
