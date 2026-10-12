#!/usr/bin/env bash
# Sourced by check_<library>.sh and the self test. run_bounded <seconds> <name> <command...>:
# runs the command in its own process group and, past the limit, kills the group and every descendant
# (a test host that left the group included). Returns the command code, or 124 on the limit.

descendants() {
  local child
  for child in $(pgrep -P "$1" 2>/dev/null); do
    echo "$child"
    descendants "$child"
  done
}

kill_tree() {
  local pid="$1" tree
  tree="$(descendants "$pid" | tr '\n' ' ')"
  kill -TERM -- "-$pid" $tree 2>/dev/null || true
  kill -TERM "$pid" 2>/dev/null || true
  sleep 2
  kill -KILL -- "-$pid" $tree 2>/dev/null || true
  kill -KILL "$pid" 2>/dev/null || true
}

run_bounded() {
  local limit="$1" name="$2" pid watcher code marker
  shift 2
  marker="$(mktemp)"
  set -m
  "$@" &
  pid=$!
  set +m
  ( sleep "$limit"; echo expired > "$marker"; kill_tree "$pid" ) &
  watcher=$!
  code=0
  wait "$pid" || code=$?
  kill "$watcher" 2>/dev/null || true
  pkill -P "$watcher" 2>/dev/null || true
  wait "$watcher" 2>/dev/null || true
  if [ -s "$marker" ]; then
    rm -f "$marker"
    echo "check: step '$name' passed $limit s, its process group was killed" >&2
    return 124
  fi
  rm -f "$marker"
  return "$code"
}

# Step table engine. The library script defines `steps` (lines "lane|name|limit seconds|paths|command"), then calls
# run_checks "$@". Lane "cheap" runs first, serial, and stops everything on a failure. The other lanes run in parallel.
# --fast keeps the steps whose paths prefix a changed file ("*" always): the same commands, a subset of the full run.
# --list prints the selected steps. --only <lane> runs one lane.

changed_files() {
  local base
  base="$(git -C "$root" merge-base HEAD master 2>/dev/null || echo HEAD)"
  { git -C "$root" diff --name-only "$base"; git -C "$root" ls-files -m -o --exclude-standard; } | sort -u
}

step_selected() {
  local paths="$1" prefix file prefixes
  [ "$fast" -eq 0 ] && return 0
  read -ra prefixes <<< "$paths"
  for prefix in "${prefixes[@]}"; do
    [ "$prefix" = "*" ] && return 0
    while IFS= read -r file; do
      case "$file" in "$prefix"*) return 0 ;; esac
    done <<< "$changed"
  done
  return 1
}

run_step() {
  local lane name limit paths command log start code waited
  IFS='|' read -r lane name limit paths command <<< "$1"
  log="$logdir/$name.log"
  start=$SECONDS
  code=0
  run_bounded "$limit" "$name" bash -c 'cd -- "$1" && shift && eval "$1"' _ "$root" "$command" > "$log" 2>&1 || code=$?
  waited="$(sed -n 's/.*got slot[0-9]* after \([0-9]*\)s.*/\1/p' "$log" | paste -sd+ - | bc 2> /dev/null)"
  printf '%-8s %-20s %-8s %4ss (queue for a build slot %ss)\n' "$lane" "$name" "$([ "$code" -eq 0 ] && echo ok || echo "FAIL($code)")" "$((SECONDS - start))" "${waited:-0}"
  if [ "$code" -ne 0 ]; then
    tail -25 "$log" | sed 's/^/    | /'
    echo "    log: $log"
  fi
  return "$code"
}

run_lane() {
  local lane="$1" line step_lane name rest
  for line in "${selected[@]}"; do
    IFS='|' read -r step_lane name rest <<< "$line"
    [ "$step_lane" = "$lane" ] || continue
    run_step "$line" || return 1
  done
}

run_checks() {
  fast=0; only=""; local list=0 line lane lane_name paths
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --fast) fast=1 ;;
      --list) list=1 ;;
      --only) only="${2:?--only needs a lane}"; shift ;;
      *) echo "usage: $0 [--fast] [--list] [--only <lane>]" >&2; return 2 ;;
    esac
    shift
  done
  changed=""
  [ "$fast" -eq 1 ] && changed="$(changed_files)"
  selected=()
  for line in "${steps[@]}"; do
    IFS='|' read -r lane lane_name _ paths _ <<< "$line"
    [ -n "$only" ] && [ "$lane" != "$only" ] && continue
    step_selected "$paths" && selected+=("$line")
  done
  if [ "$list" -eq 1 ]; then
    printf '%s\n' "${selected[@]}"
    return 0
  fi
  logdir="$root/target/check-logs"
  mkdir -p "$logdir"
  local start=$SECONDS lanes=() pids=() failed=0 i
  while IFS= read -r lane; do lanes+=("$lane"); done < <(printf '%s\n' "${selected[@]}" | cut -d'|' -f1 | awk '!seen[$0]++')
  if printf '%s\n' "${lanes[@]}" | grep -qx cheap; then
    run_lane cheap || { echo "check: cheap lane failed, nothing was built"; return 1; }
  fi
  for lane in "${lanes[@]}"; do
    [ "$lane" = cheap ] && continue
    run_lane "$lane" > "$logdir/lane-$lane.out" 2>&1 &
    pids+=("$!")
  done
  for i in "${!pids[@]}"; do
    wait "${pids[$i]}" || failed=1
  done
  for lane in "${lanes[@]}"; do
    [ -f "$logdir/lane-$lane.out" ] && cat "$logdir/lane-$lane.out"
  done
  echo "check: $([ "$failed" -eq 0 ] && echo ok || echo FAILED) in $((SECONDS - start)) s, load $(uptime | sed 's/.*averages*: //')"
  return "$failed"
}
