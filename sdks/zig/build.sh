#!/bin/sh
# Builds a Zig app:  sdks/zig/build.sh app.zig [out.wasm]
set -eu
src="$1"
out="${2:-${src%.zig}.wasm}"
here=$(cd "$(dirname "$0")" && pwd)
${ZIG:-zig} build-exe -target wasm32-freestanding -O ReleaseSmall -fno-entry -rdynamic -fstrip \
	--dep qa "-Mroot=$src" "-Mqa=$here/qa.zig" "-femit-bin=$out"
rm -f "${out%.wasm}.wasm.o"
echo "built $out ($(wc -c <"$out") bytes)"
