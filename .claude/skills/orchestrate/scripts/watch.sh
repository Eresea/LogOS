#!/usr/bin/env bash
# Poll GitHub for branch-head and PR-state changes; exit on a change that persists
# across two polls, or after MAX seconds. Issue edits and comments are ignored so the
# manager's own writes never wake it.
#
# Env: REPO (owner/name, default: origin's GitHub repo), MIN_PR (default 1),
#      INTERVAL seconds (default 120), MAX seconds (default 3000).
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
REPO=${REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)}
MIN_PR=${MIN_PR:-1}; INTERVAL=${INTERVAL:-120}; MAX=${MAX:-3000}
[ -n "$REPO" ] || { echo "WATCH ERROR: cannot resolve REPO"; exit 2; }

snap() {
  local heads prs
  heads=$(timeout 60 git ls-remote --heads origin 2>/dev/null | sort) || return 1
  prs=$(timeout 60 gh pr list --repo "$REPO" --state all --limit 50 \
      --json number,state,headRefName,headRefOid,mergedAt \
      --jq ".[] | select(.number>=$MIN_PR) | \"PR #\(.number) \(.state) \(.headRefName) \(.headRefOid[0:8]) merged=\(.mergedAt)\"" \
      2>/dev/null | sort) || return 1
  # A snapshot without main or without any PR line is a network glitch, not a change.
  echo "$heads" | grep -q 'refs/heads/main' || return 1
  echo "$prs" | grep -q '^PR #' || return 1
  printf '%s\n%s\n' "$heads" "$prs"
}

start=$(date +%s)
base=""
while [ -z "$base" ]; do
  base="$(snap)" || base=""
  [ -n "$base" ] || sleep 10
  [ $(( $(date +%s) - start )) -ge "$MAX" ] && { echo "WATCH TIMEOUT (no valid baseline)"; exit 0; }
done
while [ $(( $(date +%s) - start )) -lt "$MAX" ]; do
  sleep "$INTERVAL"
  now="$(snap)" || continue
  [ "$now" = "$base" ] && continue
  sleep 20
  again="$(snap)" || continue
  if [ "$again" = "$now" ]; then
    echo "CHANGE DETECTED $(date -Iseconds)"
    diff <(echo "$base") <(echo "$again") | grep '^[<>]'
    exit 0
  fi
done
echo "WATCH TIMEOUT $(date -Iseconds), no change"
