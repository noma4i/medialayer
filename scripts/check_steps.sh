#!/usr/bin/env bash
# Sourced by check_<library>.sh and the self test. run_bounded <seconds> <name> <command...>:
# runs the command in its own process group and, past the limit, kills the group and every descendant
# (a test host that left the group included). Returns the command code, or 124 on the limit.

# shellcheck source=gradle_host_guard.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/gradle_host_guard.sh"

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

# Step table engine. A step with a build slot gets its limit inside the slot ({limit} in the command, see check_bounded.sh)
# and the limit plus CHECK_QUEUE_ALLOWANCE (3600 s) outside, so the wait for a slot never counts as test time.
# The library script defines `steps` (lines "lane|name|limit seconds|paths|command"), then calls
# run_checks "$@". Lanes run one after another (never 2 build slot commands of one check): cheap first, table order, final last, the first red lane stops the run.
# --fast keeps the steps whose paths prefix a changed file ("*" always): the same commands, a subset of the full run.
# Lane "final" runs last, only when every lane passed (checks of their outputs). --list prints the selected steps. --only <lane> runs one lane.

# CPU seconds of processes the waited-children counter (`times`) cannot see, sampled once per second: processes below a
# java process of this check (single-use daemon, test executors) and the Gradle and Kotlin daemons that carry this
# worktree's check-owner-<id> tag in a JVM option (-XX:ErrorFile, Gradle drops plain -D options from the daemon command line), with their children. A daemon of another worktree carries another id and is
# not counted. The highest CPU time seen per pid counts, the CPU a daemon had before the run is subtracted.
counted_processes_cpu() {
  ps -axo pid=,ppid=,cputime=,command= | awk -v root="$$" -v owner="check-owner-${CHECK_OWNER_ID:-none}-" '
    { parent[$1] = $2; n = split($3, a, ":"); sec[$1] = a[n] + 60 * a[n - 1] + (n > 2 ? 3600 * a[n - 2] : 0)
      isjava[$1] = ($0 ~ /\/java( |$)/) ? 1 : 0
      tagged[$1] = ($0 ~ /GradleDaemon|KotlinCompileDaemon/ && index($0, owner) > 0) ? 1 : 0 }
    END {
      for (pid in parent) {
        inside = 0; below = 0; under_tagged = 0; up = parent[pid]
        for (hops = 0; up > 1 && hops < 30; hops++) { if (up == root) inside = 1; if (isjava[up]) below = 1; if (tagged[up]) under_tagged = 1; up = parent[up] }
        if ((inside && below) || tagged[pid] || under_tagged) print pid, sec[pid]
      }
    }'
}

sample_java_cpu() {
  local file="$1"
  while :; do
    counted_processes_cpu >> "$file"
    sleep 1
  done
}

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
  command="${command//\{limit\}/$limit}"
  outer="$limit"
  [[ "$command" == *build-slot* ]] && outer=$((limit + ${CHECK_QUEUE_ALLOWANCE:-3600}))
  run_bounded "$outer" "$name" bash -c 'cd -- "$1" && shift && eval "$1"' _ "$root" "$command" > "$log" 2>&1 || code=$?
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
  for line in "${selected[@]}"; do
    case "$line" in *gradle-step* | *gradlew* | */bin/gradle\ *) IFS='|' read -r _ lane_name _ <<< "$line"; refuse_gradle_on_darwin "step $lane_name" || return $? ;; esac
  done
  logdir="$root/target/check-logs"
  mkdir -p "$logdir"
  local start=$SECONDS lanes=() ordered=() failed=0 cpu sampled before sampler cpu_children daemon
  CHECK_OWNER_ID="$(printf '%s' "$root" | shasum | cut -c1-12)"
  export CHECK_OWNER_ID
  sampled="$(mktemp)"
  before="$(mktemp)"
  counted_processes_cpu > "$before"
  export CHECK_SAMPLES_FILE="$sampled"
  sample_java_cpu "$sampled" &
  sampler=$!
  while IFS= read -r lane; do lanes+=("$lane"); done < <(printf '%s\n' "${selected[@]}" | cut -d'|' -f1 | awk '!seen[$0]++')
  # One lane at a time: the check never holds more than one build slot command, the first red lane stops the run.
  ordered=(cheap)
  for lane in "${lanes[@]}"; do
    [ "$lane" = cheap ] || [ "$lane" = final ] || ordered+=("$lane")
  done
  ordered+=(final)
  for lane in "${ordered[@]}"; do
    [[ " ${lanes[*]} " == *" $lane "* ]] || continue
    run_lane "$lane" || { failed=1; break; }
  done
  times > "$logdir/times.txt"
  cpu_children="$(tail -1 "$logdir/times.txt" | tr 'ms' '  ' | awk '{printf "%d", $1 * 60 + $2 + $3 * 60 + $4}')"
  kill "$sampler" 2> /dev/null; wait "$sampler" 2> /dev/null
  daemon="$(awk -v before="$before" 'BEGIN { while ((getline line < before) > 0) { split(line, p, " "); was[p[1]] = p[2] } }
    { if ($2 > best[$1]) best[$1] = $2 }
    END { for (pid in best) total += best[pid] - (pid in was ? was[pid] : 0); printf "%d", total }' "$sampled")"
  rm -f "$sampled" "$before"
  cpu=$((cpu_children + daemon))
  echo "check: $([ "$failed" -eq 0 ] && echo ok || echo FAILED), at least $cpu CPU seconds, a lower bound from 1 s samples ($cpu_children in waited children, $daemon in daemons and processes below java), ${#selected[@]} steps selected, wall $((SECONDS - start)) s, load $(uptime | sed 's/.*averages*: //')"
  return "$failed"
}
