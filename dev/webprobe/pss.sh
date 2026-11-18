#!/usr/bin/env bash
# Sums the proportional memory (PSS) of every process whose command line matches the pattern, in MB.
# Usage: dev/webprobe/pss.sh <pattern>
pattern=$1
total=0
for pid in $(pgrep -f -- "$pattern"); do
    kb=$(awk '/^Pss:/ {print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null) || continue
    total=$((total + ${kb:-0}))
    printf '%8d kB  %s\n' "${kb:-0}" "$(tr '\0' ' ' < "/proc/$pid/cmdline" | cut -c1-90)"
done
echo "total: $((total / 1024)) MB"
