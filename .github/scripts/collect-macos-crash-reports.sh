#!/bin/bash
# Post-failure collection only. Never reruns a test or collects core dumps.
set -u
stamp=$1
destination=$2
reports=${3:-"$HOME/Library/Logs/DiagnosticReports"}
mkdir -p "$destination"
manifest="$destination/crash-reports.tsv"
printf 'original_bytes\tcopied_bytes\ttruncated\tmtime_epoch\tfilename\n' > "$manifest"
if [[ ! -f "$stamp" ]]; then
    printf 'Missing pre-test timestamp marker\n' > "$destination/collection-errors.log"
    exit 0
fi
start=$SECONDS
while :; do
    found=0
    for file in "$reports"/{integration-*,symslice-mcp*}.{ips,crash} "$reports"/Retired/{integration-*,symslice-mcp*}.{ips,crash}; do
        [[ -f "$file" && ! -L "$file" && "$file" -nt "$stamp" ]] && found=1
    done
    [[ "$found" -eq 1 || $((SECONDS - start)) -ge 10 ]] && break
    sleep 1
done
count=0
total=0
for file in "$reports"/{integration-*,symslice-mcp*}.{ips,crash} "$reports"/Retired/{integration-*,symslice-mcp*}.{ips,crash}; do
    [[ -f "$file" && ! -L "$file" && "$file" -nt "$stamp" ]] || continue
    [[ "$count" -ge 20 || "$total" -ge 16777216 ]] && break
    size=$(wc -c < "$file" 2>> "$destination/collection-errors.log") || continue
    copied=$((size))
    [[ "$copied" -gt 2097152 ]] && copied=2097152
    remaining=$((16777216 - total))
    [[ "$copied" -gt "$remaining" ]] && copied=$remaining
    name="${count}-${file##*/}"
    if [[ "$OSTYPE" == darwin* ]]; then
        modified=$(stat -f '%m' "$file" 2>> "$destination/collection-errors.log") || modified=UNKNOWN
    else
        modified=$(stat -c '%Y' "$file" 2>> "$destination/collection-errors.log") || modified=UNKNOWN
    fi
    if head -c "$copied" "$file" > "$destination/$name" 2>> "$destination/collection-errors.log"; then
        actual=$(wc -c < "$destination/$name")
        truncated=false
        [[ "$actual" -lt "$size" ]] && truncated=true
        printf '%s\t%s\t%s\t%s\t%s\n' "$size" "$actual" "$truncated" "$modified" "$name" >> "$manifest"
        total=$((total + actual))
        count=$((count + 1))
    fi
done
printf 'reports=%s bytes=%s wait_seconds=%s limits=20files,2MiB_each,16MiB_total\n' "$count" "$total" "$((SECONDS - start))" > "$destination/collection-summary.txt"
