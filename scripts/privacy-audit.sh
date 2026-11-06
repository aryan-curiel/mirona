#!/usr/bin/env bash
# Gate 3 privacy audit (NGR-171). See docs/privacy-audit.md.
#
#   scripts/privacy-audit.sh            full run: CLI + MCP on a throwaway profile, privacy on
#   scripts/privacy-audit.sh watch PID [SECONDS]
#                                       watch a running process (the app) for SECONDS (60)
#
# Every socket the audited processes open is sampled (lsof, every 0.1 s). Any connection to
# a host other than this machine fails the audit, as does a secret said in the test call
# showing up in a summary or an MCP result. macOS only (`say`, `afconvert`).
set -euo pipefail
cd "$(dirname "$0")/.."

MODELS=${MIRONA_MODELS_DIR:-"$HOME/Library/Application Support/com.nagare.mirona/models"}
SECRETS=("Tulip" "4471" "182,000" "182000")

# Remote endpoints of a process's sockets, one per line (listening sockets have none).
sample() { lsof -nP -a -p "$1" -i -F n 2>/dev/null | sed -n 's/^n//p' | grep -- '->' || true; }

# Watch PID until it exits (or for $2 seconds); append its connections to $3.
watch_pid() {
  local pid=$1 secs=${2:-0} log=$3 start=$SECONDS
  while kill -0 "$pid" 2>/dev/null; do
    sample "$pid" >>"$log"
    if [ "$secs" -gt 0 ] && [ $((SECONDS - start)) -ge "$secs" ]; then break; fi
    sleep 0.1
  done
}

# Connections whose remote end isn't loopback.
offending() {
  sort -u "$1" | awk -F'->' '{print $2}' |
    grep -vE '^(127\.[0-9.]+|\[::1\]|localhost):[0-9]+$' || true
}

verdict() {
  local log=$1 bad
  bad=$(offending "$log")
  echo "connections seen: $(sort -u "$log" | wc -l | tr -d ' ') (all listed in $log)"
  if [ -n "$bad" ]; then
    echo "FAIL: connections leaving this machine:"
    echo "$bad"
    return 1
  fi
  echo "PASS: no connection left this machine"
}

if [ "${1:-}" = watch ]; then
  pid=${2:?usage: privacy-audit.sh watch PID [SECONDS]}
  log=$(mktemp -t mirona-net)
  echo "watching pid $pid for ${3:-60} s…"
  watch_pid "$pid" "${3:-60}" "$log"
  verdict "$log"
  exit
fi

[ -d "$MODELS" ] || { echo "no models in $MODELS (scripts/fetch-models.sh)"; exit 1; }
curl -sf -m 2 http://localhost:11434/api/tags >/dev/null ||
  { echo "Ollama isn't running: privacy mode summarizes and classifies with it"; exit 1; }

cargo build -q -p mirona-cli
BIN=$PWD/target/debug/mirona
TMP=$(mktemp -d -t mirona-audit)
APP=$TMP/app
NET=$TMP/net.log
OUT=$TMP/out.txt
: >"$NET" >"$OUT"
mkdir -p "$APP"
cat >"$APP/config.toml" <<EOF
privacy = true
meetings_dir = "$TMP/meetings"
models_dir = "$MODELS"
EOF
export MIRONA_APP_DIR=$APP

echo "1/4 a test call with a password and a salary in it"
say -v Samantha -o "$TMP/call.aiff" \
  "Thanks for joining. We agreed to ship the onboarding redesign on Monday. \
   I'll send the revised plan to the team by Thursday. \
   By the way, the staging password is Tulip 4471. \
   And for the offer, the salary would be 182,000 a year. \
   Let's meet again next week to review the numbers."
afconvert -f WAVE -d LEI16@16000 -c 1 "$TMP/call.aiff" "$TMP/call.wav"

run() { # run and watch a CLI command, keeping its stdout
  "$BIN" "$@" >>"$OUT" 2>>"$TMP/err.txt" &
  local pid=$!
  watch_pid "$pid" 0 "$NET"
  wait "$pid"
}

echo "2/4 import: transcribe, tag, diarize, summarize, embed (privacy on)"
run import "$TMP/call.wav"
ID=$("$BIN" list | awk 'NR==1 {print $1}')
echo "    meeting $ID"

echo "3/4 search and show"
run search onboarding
run search staging password
run show "$ID"

echo "4/4 MCP: every read tool, and ask_meeting"
{
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"audit","version":"1"}}}'
  echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_meetings","arguments":{"query":"password"}}}'
  echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_meeting","arguments":{"id":"'"$ID"'"}}}'
  echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_transcript","arguments":{"id":"'"$ID"'"}}}'
  echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_action_items","arguments":{}}}'
  echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"ask_meeting","arguments":{"question":"What is the staging password and the salary?"}}}'
  sleep 60 # ask_meeting may wait on the local LLM; the server answers in order
} | "$BIN" mcp >>"$OUT" 2>>"$TMP/err.txt" &
MCP=$!
# The pipeline's last process is the server; watch it until it answered all six.
SERVER=$(pgrep -n -f "$BIN mcp" || echo "$MCP")
for _ in $(seq 1 600); do
  sample "$SERVER" >>"$NET"
  [ "$(grep -c '"id":6' "$OUT" || true)" -ge 1 ] && break
  sleep 0.1
done
kill "$MCP" 2>/dev/null || true
pkill -f "$BIN mcp" 2>/dev/null || true

echo
status=0
verdict "$NET" || status=1
summary=$("$BIN" show "$ID" | sed -n '1,/^## Transcript/p')
leaks=()
for s in "${SECRETS[@]}"; do
  if grep -qiF -- "$s" "$OUT" || grep -qiF -- "$s" <<<"$summary"; then leaks+=("$s"); fi
done
if [ ${#leaks[@]} -gt 0 ]; then
  echo "FAIL: secrets in summaries or MCP results: ${leaks[*]} (see $OUT)"
  echo "      (if the transcript line isn't tagged sensitive, the classifier missed it:"
  echo "       \`$BIN show $ID\`)"
  status=1
else
  echo "PASS: no secret in summaries, search or MCP results"
fi
grep -q '"id":6' "$OUT" || { echo "FAIL: the MCP server didn't answer every call"; status=1; }
echo "output: $OUT · profile: $TMP"
exit $status
