#!/bin/sh
# Builds the website into site/dist: the landing page, install.sh, the
# runtime binary, and the example apps packed as bundles.
#
#   site/build.sh [base-url]    (default http://localhost:8765)
#   python3 -m http.server 8765 -d site/dist
set -eu

base="${1:-http://localhost:8765}"
base="${base%/}"
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/site/dist"
qa="$root/target/release/qa"

cargo build --release -p qa --manifest-path "$root/Cargo.toml"
rm -rf "$out"
mkdir -p "$out/apps"

# slug|name|command|description
apps='widgets|Widget Gallery|widgets|Every built-in component on two tabs: buttons, inputs, lists, tabs and images.
slack|Slack|qa-slack|A small Slack client with threads, reactions and live updates. Needs a Slack token.
counter|Counter|counter|A number and two buttons. The smallest useful app.
fetch|Fetch|qa-fetch|Asks for internet access, then loads a GitHub repo over HTTPS and shows the JSON.
sandbox|Sandbox|sandbox|Tries the things an app can'"'"'t do on its own: files, folders, network, memory and CPU.
inbox|Inbox|inbox|Sends and receives messages between apps.'

echo "$apps" | while IFS='|' read -r slug name command desc; do
	src="$root/examples/$slug"
	(cd "$src" && cargo build --release --target wasm32-unknown-unknown -q)
	stage=$(mktemp -d)
	cp "$src/target/wasm32-unknown-unknown/release/$slug.wasm" "$stage/app.wasm"
	[ -f "$src/icon.png" ] && cp "$src/icon.png" "$stage/"
	[ -d "$src/assets" ] && cp -r "$src/assets" "$stage/"
	version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$src/Cargo.toml" | head -1)
	python3 -c 'import json,sys; print(json.dumps(dict(zip(["name","version","description","command"], sys.argv[1:]))))' \
		"$name" "$version" "$desc" "$command" >"$stage/app.json"
	"$qa" pack "$stage" --out "$out/apps/$slug" >/dev/null
	rm -rf "$stage"
	echo "packed $name $version"
done

cp "$qa" "$out/qa-x86_64-linux"
sha=$(sha256sum "$out/qa-x86_64-linux" | cut -d' ' -f1)
sed -e "s|@BASE@|$base|g" -e "s|@SHA256@|$sha|g" "$root/site/install.sh" >"$out/install.sh"
cp "$root/site/index.html" "$out/"
[ -f "$root/examples/widgets/icon.png" ] && cp "$root/examples/widgets/icon.png" "$out/icon.png"
echo "built $out for $base"
