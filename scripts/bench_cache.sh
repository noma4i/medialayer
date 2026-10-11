#!/usr/bin/env bash
# Interleaved cache read benchmark: baseline and candidate alternate in the same window, 5 repeats each.
# Usage: bench_cache.sh <read|proxy> <baseline_bin> <baseline_dir> <candidate_bin> <candidate_dir> [cache_bench options...]
# The binaries are built from the `cache_bench` example (cargo build --release --example cache_bench) and filled with `fill`.
# A directory that a run changes (eviction, legacy conversion) must be a fresh copy: use `cp -cR` before each call.
set -euo pipefail

mode="$1"; base_bin="$2"; base_dir="$3"; cand_bin="$4"; cand_dir="$5"; shift 5
repeats="${BENCH_REPEATS:-5}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for round in $(seq 1 "$repeats"); do
  "$base_bin" "$mode" --dir "$base_dir" --repeats 1 "$@" | grep '^RESULT' >> "$work/baseline.txt"
  "$cand_bin" "$mode" --dir "$cand_dir" --repeats 1 "$@" | grep '^RESULT' >> "$work/candidate.txt"
done

python3 -I - "$work/baseline.txt" "$work/candidate.txt" <<'PY'
import re, sys, statistics
for name, path in (("baseline", sys.argv[1]), ("candidate", sys.argv[2])):
    columns = {}
    for line in open(path):
        for key, value in re.findall(r"(\w+)=([0-9.]+)", line):
            columns.setdefault(key, []).append(float(value))
    for key, values in columns.items():
        if key in ("repeat",):
            continue
        print(f"{name:9} {key:22} min={min(values):12.1f} median={statistics.median(values):12.1f} max={max(values):12.1f} n={len(values)}")
PY
