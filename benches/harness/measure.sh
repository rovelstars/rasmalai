#!/usr/bin/env bash
# measure.sh - peak RSS and wall-time wrapper for a single command.
#
# Usage: measure.sh <stdout-file> <command> [args...]
#
# Runs <command> with stdout redirected to <stdout-file>, then prints one
# JSON line to stdout:
#   {"elapsed_ns":N,"max_rss_bytes":N,"user_time_ns":N,"sys_time_ns":N,"exit_code":N}
#
# Method: exactly one code path. A tiny compiled helper
# (harness/measure.c, ~15 KB RSS, no heap before fork) runs fork/execvp
# plus wait4 and reports the child's own rusage. There is no GNU time
# branch and no Python branch: two parents that disagree on wall clock
# and RSS cannot silently produce incomparable rows.
set -u

if [ "$#" -lt 2 ]; then
    echo "usage: measure.sh <stdout-file> <command> [args...]" >&2
    exit 2
fi

here="$(dirname "$0")"
helper="$here/measure"
src="$here/measure.c"

if [ ! -x "$helper" ] || [ "$src" -nt "$helper" ]; then
    cc="${CC:-cc}"
    if ! "$cc" -O2 -o "$helper" "$src" 2>/dev/null; then
        echo "measure.sh: cannot build $helper from $src with $cc" >&2
        exit 2
    fi
fi

exec "$helper" "$@"
