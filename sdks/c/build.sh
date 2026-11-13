#!/bin/sh
# Builds a C app:  sdks/c/build.sh app.c [out.wasm]
# Uses clang if it can link wasm (needs wasm-ld), else `zig cc`.
set -eu
src="$1"
out="${2:-${src%.c}.wasm}"
flags="-Oz -nostdlib -fno-builtin -Wall -Wl,--no-entry -Wl,--strip-all -Wl,-z,stack-size=65536"
if command -v wasm-ld >/dev/null 2>&1; then
	clang --target=wasm32-unknown-unknown $flags -o "$out" "$src"
else
	${ZIG:-zig} cc -target wasm32-freestanding $flags -o "$out" "$src"
fi
echo "built $out ($(wc -c <"$out") bytes)"
