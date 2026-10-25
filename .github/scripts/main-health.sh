#!/usr/bin/env bash
# Report main-branch health for affected-only CI.
#
# Outputs one line on stdout:
#   MAIN_GREEN              last completed main run succeeded: affected runs proceed.
#   MAIN_RED_SINCE_<sha>   last completed main run failed: caller runs FULL and
#                           annotates "main red since <sha>".
#   MAIN_GREEN              on ANY error (no token, API failure, no runs yet):
#                           fail open, never block the suite on this check.
#
# Read-only: uses `gh run list` against the current repo. Needs no scopes
# beyond the default GITHUB_TOKEN.
set -u

fail_open() {
	echo "MAIN_GREEN"
	exit 0
}

command -v gh >/dev/null 2>&1 || fail_open
command -v jq >/dev/null 2>&1 || fail_open

RUNS="$(gh run list --workflow CI --branch main --limit 20 --json databaseId,conclusion,headSha,url \
	--jq '.[] | select(.conclusion == "success" or .conclusion == "failure")' 2>/dev/null)" \
	|| fail_open
[ -n "$RUNS" ] || fail_open

LATEST="$(echo "$RUNS" | jq -r '.conclusion' 2>/dev/null | head -1)" || fail_open
if [ "$LATEST" = "success" ]; then
	echo "MAIN_GREEN"
	exit 0
fi
if [ "$LATEST" != "failure" ]; then
	fail_open
fi

SINCE="$(echo "$RUNS" | jq -s -r '. as $runs
	| ([range(0; ($runs | length)) | select($runs[.] | .conclusion != "failure")]
		| .[0] // ($runs | length)) as $n
	| if $n == 0 then empty else $runs[$n - 1] | .headSha end
' 2>/dev/null)" || fail_open
[ -n "$SINCE" ] || fail_open
echo "MAIN_RED_SINCE_${SINCE}"
