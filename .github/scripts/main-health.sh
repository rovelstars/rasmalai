#!/usr/bin/env bash
# Report main-branch health for affected-only CI.
#
# Outputs two lines on stdout:
#   line 1: MAIN_GREEN | MAIN_RED_SINCE_<sha> (fail open to MAIN_GREEN)
#   line 2: INFLIGHT_<sha> (newest in-progress CI run on main) | INFLIGHT_NONE
# Line 2 is informational only: it never forces FULL, so busy periods cannot
# turn every PR run into a full run. Callers annotate it with ::notice::.
#
# Read-only: uses `gh run list` against the current repo. Needs no scopes
# beyond the default GITHUB_TOKEN.
set -u

fail_open() {
	echo "MAIN_GREEN"
	echo "INFLIGHT_NONE"
	exit 0
}

# Newest in-progress CI run on main (short sha), or NONE. Informational
# only; never gates anything. Fails closed to NONE, never to a sha.
in_flight_sha() {
	local sha
	sha="$(gh run list --workflow CI --branch main --limit 5 \
		--json headSha,status --jq '.[] | select(.status != "completed") | .headSha' 2>/dev/null \
		| head -1 | cut -c1-7)" || { echo "NONE"; return 0; }
	[ -n "$sha" ] && echo "$sha" || echo "NONE"
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
	echo "INFLIGHT_$(in_flight_sha)"
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
echo "INFLIGHT_$(in_flight_sha)"
exit 0
