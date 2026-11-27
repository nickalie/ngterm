#!/usr/bin/env bash
# Tests scripts/ci-host-monitor.sh on stand-ins for the macOS tools it reads (sysctl, vm_stat, memory_pressure, df, ps,
# top), on any host: each line has its load and memory, the first line and each line under the free limit name the
# processes with the largest footprint (top's MEM, compressed pages included) and the most CPU, and the other lines
# name none.
#
# Usage: scripts/test-ci-host-monitor.sh
set -euo pipefail

monitor="$(cd "$(dirname "$0")" && pwd)/ci-host-monitor.sh"
work="$(mktemp -d)"
pid=""
trap '[[ -z "$pid" ]] || kill "$pid" 2>/dev/null; rm -rf "$work"' EXIT
mkdir "$work/bin"

stub() {
  printf '#!/usr/bin/env bash\n%s\n' "$2" >"$work/bin/$1"
  chmod +x "$work/bin/$1"
}
stub sysctl 'case "$2" in
  hw.memsize) echo 7516192768 ;;
  hw.ncpu) echo 3 ;;
  vm.loadavg) echo "{ 594.78 447.03 323.17 }" ;;
  vm.swapusage) echo "total = 1024.00M  used = 532.19M  free = 491.81M  (encrypted)" ;;
esac'
stub vm_stat 'printf "Pageins:  100.\nPageouts:  5.\nSwapins:  0.\nSwapouts:  0.\nCompressions:  7.\nDecompressions:  3.\n"'
stub df 'printf "Filesystem 1M-blocks Used Available\n/dev/disk1 100000 64171 35829\n"'
# the free percentage of each sample in turn: the first, one under the limit, then one above it
stub memory_pressure "n=\$(cat '$work/samples' 2>/dev/null || echo 0); echo \$((n + 1)) >'$work/samples'
free=(53 25 54)
echo \"System-wide memory free percentage: \${free[n]:-54}%\""
stub ps 'printf " 12.0 /usr/libexec/diagnosticd\n 49.1 /System/Library/ReportCrashService\n"'
stub top 'cat <<TOP
Processes: 512 total, 4 running, 508 sleeping, 2843 threads
PhysMem: 6800M used (1900M wired, 2100M compressor), 300M unused.

MEM    COMMAND
1712M+ NgTerm
415M   BackgroundShortc
2G-    healthappd
512K   logd
TOP'

PATH="$work/bin:$PATH" "$monitor" "$work/load.log" 1 &
pid=$!
samples() { if [[ -f "$work/load.log" ]]; then grep -c '^20' "$work/load.log" || true; else echo 0; fi; }
for _ in $(seq 1 100); do
  (($(samples) >= 3)) && break
  sleep 0.1
done
kill "$pid"
wait "$pid" 2>/dev/null || true
pid=""

failures=0
check() {
  if eval "$2"; then printf 'ok   %s\n' "$1"; else
    printf 'FAIL %s\n%s\n' "$1" "$(cat "$work/load.log")" >&2
    failures=$((failures + 1))
  fi
}
line() { grep '^20' "$work/load.log" | sed -n "${1}p"; }
processes=' | mem: NgTerm=1712M BackgroundShortc=415M healthappd=2048M logd=0M | cpu: ReportCrashService=49.1% diagnosticd=12.0%'

check "the log starts with the machine" \
  '[[ "$(head -n1 "$work/load.log")" == "host: memory 7168 MB, 3 CPUs, disk free 35829 MB" ]]'
check "a line has the load, free memory, swap and paging" \
  '[[ "$(line 1)" == *" load=594.78,447.03,323.17 free=53% swap=532.19M pageins=+0 "* ]]'
check "the first line names the processes by footprint and CPU" '[[ "$(line 1)" == *"$processes" ]]'
check "a line under the free limit names them" '[[ "$(line 2)" == *" free=25% "*"$processes" ]]'
check "a line above the limit, not the sixth, names none" '[[ "$(line 3)" == *" free=54% "* && "$(line 3)" != *"|"* ]]'

((failures == 0)) || exit 1
