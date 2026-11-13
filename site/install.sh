#!/bin/sh
# Installs the Quick Apps runtime as ~/.local/bin/qa (Linux or macOS), makes qa: links open
# with it, and then opens the app at $1 if one is given. Needs no root.
#
#   curl -fsSL @BASE@/install.sh | sh
#   curl -fsSL @BASE@/install.sh | sh -s -- https://example.com/notes/app.json
set -eu

base="${QA_BASE:-@BASE@}"
bin="$HOME/.local/bin"

case "$(uname -s)-$(uname -m)" in
Linux-x86_64) target=x86_64-linux sha="@SHA256_X86_64_LINUX@" ;;
Darwin-x86_64) target=x86_64-darwin sha="@SHA256_X86_64_DARWIN@" ;;
Darwin-arm64) target=aarch64-darwin sha="@SHA256_AARCH64_DARWIN@" ;;
*) target= sha=none ;;
esac
if [ "$sha" = none ]; then
	echo "quick-apps: there is no build for $(uname -s) $(uname -m) yet" >&2
	exit 1
fi

mkdir -p "$bin"
tmp=$(mktemp "$bin/.qa.XXXXXX")
trap 'rm -f "$tmp"' EXIT
echo "quick-apps: downloading $base/qa-$target"
if command -v curl >/dev/null 2>&1; then
	curl -fsSL "$base/qa-$target" -o "$tmp"
else
	wget -qO "$tmp" "$base/qa-$target"
fi
if command -v sha256sum >/dev/null 2>&1; then
	got=$(sha256sum "$tmp" | cut -d' ' -f1)
else
	got=$(shasum -a 256 "$tmp" | cut -d' ' -f1)
fi
if [ "$got" != "$sha" ]; then
	echo "quick-apps: the download doesn't match its checksum" >&2
	exit 1
fi
chmod 755 "$tmp"
mv "$tmp" "$bin/qa"
trap - EXIT
echo "quick-apps: installed $bin/qa"
"$bin/qa" setup

if [ $# -gt 0 ]; then
	exec "$bin/qa" open "$1"
fi
