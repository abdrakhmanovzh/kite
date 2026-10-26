#!/bin/sh
# Rebuilds and relaunches the app whenever a file in src/ changes.
# GPUI has no hot reload; an incremental rebuild takes ~10 seconds.
set -u
cd "$(dirname "$0")/.."

stamp() { find src Cargo.toml -type f -exec stat -f '%m %N' {} + | sort | md5 -q; }

pid=""
restart() {
    if cargo build --locked 2>&1 | grep -E '^(error|warning)' -A6; then :; fi
    if [ ! -x target/debug/kite ] || ! cargo build --locked -q 2>/dev/null; then
        echo "build failed; keeping the running app"
        return
    fi
    [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
    ./target/debug/kite >/dev/null 2>&1 &
    pid=$!
    echo "$(date +%H:%M:%S) running (pid $pid)"
}

trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null; exit 0' INT TERM
last=$(stamp)
restart
while sleep 1; do
    now=$(stamp)
    if [ "$now" != "$last" ]; then
        last=$now
        echo "$(date +%H:%M:%S) change detected, rebuilding…"
        restart
    fi
done
