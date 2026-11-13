#!/bin/sh
# Installs the Quick Apps runtime as ~/.local/bin/qa, makes qa: links open
# with it, and then opens the app at $1 if one is given. Needs no root.
#
#   curl -fsSL @BASE@/install.sh | sh
#   curl -fsSL @BASE@/install.sh | sh -s -- https://example.com/notes/app.json
set -eu

base="${QA_BASE:-@BASE@}"
sha="@SHA256@"
bin="$HOME/.local/bin"

case "$(uname -s)-$(uname -m)" in
Linux-x86_64) ;;
*)
	echo "quick-apps: there is only a Linux x86_64 build so far" >&2
	exit 1
	;;
esac

mkdir -p "$bin"
tmp=$(mktemp "$bin/.qa.XXXXXX")
trap 'rm -f "$tmp"' EXIT
echo "quick-apps: downloading $base/qa-x86_64-linux"
if command -v curl >/dev/null 2>&1; then
	curl -fsSL "$base/qa-x86_64-linux" -o "$tmp"
else
	wget -qO "$tmp" "$base/qa-x86_64-linux"
fi
if command -v sha256sum >/dev/null 2>&1; then
	echo "$sha  $tmp" | sha256sum -c --quiet - || {
		echo "quick-apps: the download doesn't match its checksum" >&2
		exit 1
	}
fi
chmod 755 "$tmp"
mv "$tmp" "$bin/qa"
trap - EXIT
echo "quick-apps: installed $bin/qa"
"$bin/qa" setup

if [ $# -gt 0 ]; then
	exec "$bin/qa" open "$1"
fi
