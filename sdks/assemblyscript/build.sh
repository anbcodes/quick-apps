#!/bin/sh
# Builds an AssemblyScript app:  sdks/assemblyscript/build.sh app.ts [out.wasm]
# Needs the compiler: npm install assemblyscript (or set ASC).
set -eu
src="$1"
out="${2:-${src%.ts}.wasm}"
# --runtime minimal: a small allocator, with garbage collected after each event.
# --use abort=: no `env.abort` import, which the runtime doesn't provide.
${ASC:-npx asc} "$src" -o "$out" -Oz --runtime minimal --use abort=
echo "built $out ($(wc -c <"$out") bytes)"
