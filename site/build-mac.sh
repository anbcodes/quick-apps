#!/bin/sh
# Builds the macOS runtimes (Intel and Apple silicon) on a Mac you can ssh to and copies
# them to site/mac, where build.sh picks them up. The Mac needs Xcode's command line tools
# and rustup; this script copies the sources over and doesn't install anything else.
#
#   site/build-mac.sh user@host
set -eu

host="${1:?usage: site/build-mac.sh user@host}"
root=$(cd "$(dirname "$0")/.." && pwd)
ssh_opts="-o BatchMode=yes -o RemoteCommand=none -o RequestTTY=no"
sync() { rsync -a --delete -e "ssh $ssh_opts" --exclude target --exclude .git --exclude dist --exclude node_modules --exclude 'examples/*/target' "$1/" "$host:qa-build/$2/"; }

ssh $ssh_opts "$host" 'mkdir -p qa-build' </dev/null
sync "$root" quick-apps
sync "$root/../mtui" mtui
mkdir -p "$root/site/mac"
for t in x86_64-apple-darwin aarch64-apple-darwin; do
	ssh $ssh_opts "$host" ". ~/.cargo/env; rustup target add $t >/dev/null 2>&1; cd qa-build/quick-apps && cargo build --release -p qa --target $t" </dev/null
	scp -q $ssh_opts "$host:qa-build/quick-apps/target/$t/release/qa" "$root/site/mac/qa-${t%%-apple-darwin}-darwin"
	echo "built $t"
done
