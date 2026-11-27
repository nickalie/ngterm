#!/usr/bin/env bash
# Samples a CI Mac's load into a log until it is killed, so that a job the runner gives up on mid-test leaves a record
# of what the machine was doing: ios.yml starts it in the background before the test plans (the runner kills it with
# the job's other leftover processes) and keeps the log beside the runner's own when a job fails or is cancelled.
#
# A line every INTERVAL seconds (default 10): the UTC time; `late`, how much longer than INTERVAL the sample took to
# come round, which is how long the whole machine stood still (a sleeping shell is the last thing to be held up); the
# load averages; memory_pressure's free percentage; the swap in use; and the pages paged, swapped and compressed
# since the last line (vm_stat's pageins, pageouts, swapins, swapouts, compressions, decompressions). Every sixth line,
# and each line whose free percentage is below LOW_FREE, adds the processes with the most memory and the most CPU. A
# process's memory is top's MEM, its footprint: under pressure the system compresses and swaps a process's pages, so
# its resident size falls while its footprint does not (a nightly stall showed every process under 200 MB resident at
# 25% free). It starts with the machine's memory, CPUs and free disk.
#
# Usage: scripts/ci-host-monitor.sh <log> [interval]
# macOS only.
set -uo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: scripts/ci-host-monitor.sh <log> [interval]" >&2
  exit 2
fi
LOG="$1"
INTERVAL="${2:-10}"
mkdir -p "$(dirname "$LOG")"

# vm_stat's counter named $1 (its lines read "Pageins:   123.") from the output in $2
vm_counter() {
  awk -v name="$1:" '$0 ~ "^" name { gsub(/\./, "", $NF); print $NF; exit }' <<<"$2"
}

# A line's processes are listed when its free percentage is below this.
LOW_FREE=40

# the processes with the most CPU, as "name=value%" pairs
top_cpu() {
  ps -axo "%cpu=,comm=" | sort -rn | head -6 | awk '{
    value = $1; $1 = ""; n = split(substr($0, 2), path, "/")
    printf " %s=%s%%", path[n], value
  }'
}

# the processes with the most memory, their footprint in MB, as "name=valueM" pairs, from `top -stats mem,command`
# on stdin: the rows after its MEM header, each a size with a K, M or G unit (a + or - after it when it changed) and
# the command, which top cuts to 16 characters
footprints() {
  awk '
    $1 == "MEM" { rows = 1; next }
    rows && NF >= 2 {
      size = $1; sub(/[-+]$/, "", size)
      unit = substr(size, length(size)); value = substr(size, 1, length(size) - 1) + 0
      mb = unit == "G" ? value * 1024 : unit == "K" ? value / 1024 : unit == "B" ? 0 : value
      $1 = ""; printf " %s=%dM", substr($0, 2), mb
    }'
}

# whether memory_pressure's free percentage $1 ("25%") is below LOW_FREE
is_low() {
  [[ "${1%\%}" =~ ^[0-9]+$ ]] && ((${1%\%} < LOW_FREE))
}

top_memory() {
  top -l 1 -o mem -n 6 -stats mem,command 2>/dev/null | footprints
}

{
  echo "host: memory $(($(sysctl -n hw.memsize) / 1048576)) MB, $(sysctl -n hw.ncpu) CPUs," \
    "disk free $(df -m / | awk 'NR == 2 { print $4 }') MB"
} >>"$LOG"

previous="$(vm_stat)"
expected="$(date +%s)"
count=0
while :; do
  now="$(date +%s)"
  stats="$(vm_stat)"
  line="$(date -u +%FT%TZ) late=$((now - expected))s"
  line+=" load=$(sysctl -n vm.loadavg | tr -d '{}' | awk '{ print $1 "," $2 "," $3 }')"
  free="$(memory_pressure -Q 2>/dev/null | awk -F': ' '/percentage/ { print $2 }')"
  line+=" free=$free"
  line+=" swap=$(sysctl -n vm.swapusage | awk '{ print $6 }')"
  for counter in Pageins Pageouts Swapins Swapouts Compressions Decompressions; do
    line+=" $(tr '[:upper:]' '[:lower:]' <<<"$counter")=+$(($(vm_counter "$counter" "$stats") - $(vm_counter "$counter" "$previous")))"
  done
  if ((count % 6 == 0)) || is_low "$free"; then
    line+=" | mem:$(top_memory) | cpu:$(top_cpu)"
  fi
  echo "$line" >>"$LOG"
  previous="$stats"
  count=$((count + 1))
  expected=$((now + INTERVAL))
  sleep "$INTERVAL"
done
