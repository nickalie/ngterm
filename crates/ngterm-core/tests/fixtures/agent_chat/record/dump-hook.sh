#!/bin/sh
# dump-hook.sh AGENT EVENT: save the hook payload (stdin) and the agent-related environment.
dir="$(dirname "$0")/payloads/$1"
name="$(date +%s%N)-$2"
cat > "$dir/$name.json"
env | grep -E '^(CLAUDE|CODEX)' | grep -v -E 'TOKEN|KEY|SECRET' | sort > "$dir/$name.env"
exit 0
