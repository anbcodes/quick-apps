#!/bin/sh
# Builds a TinyGo app:  sdks/tinygo/build.sh ./examples/counter [out.wasm]
# Run it from sdks/tinygo (or your own module that imports the qa package).
set -eu
pkg="$1"
out="${2:-$pkg/app.wasm}"
# wasm-unknown has no WASI. Its default collector never frees, so ask for the real one.
${TINYGO:-tinygo} build -target wasm-unknown -gc conservative -opt z -no-debug -panic trap -o "$out" "$pkg"
echo "built $out ($(wc -c <"$out") bytes)"
