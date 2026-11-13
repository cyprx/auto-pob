#!/usr/bin/env bash
# Runs the PoB comparison gate as one process per test, N at a time.
#
#   AUTO_POB_POB_DIR=<pinned PoB> scripts/gate.sh [-j N] [-f FILTER] [-t SECS]
#
# Builds the gate once, then runs every ignored gate test in its own process of the built
# binary. Tests listed in app/tests/gate-serial.txt (they share machine-wide resources) run
# one at a time after the parallel ones. Keeps target/gate-times.tsv as
# `name<TAB>seconds<TAB>ok|FAIL`, slowest first: a run replaces the rows of the tests it ran
# and keeps the others. Exits 0 only if every selected test passed
# and at least one was selected. A test still running after SECS seconds (default 300) is
# killed and counts as FAIL. Each process can peak near 1.7 GB, so keep N at 4 or below on a
# 16 GB machine.
set -uo pipefail

self=$(readlink -f "${BASH_SOURCE[0]}")
root=$(dirname "$(dirname "$self")")
serial_file="$root/app/tests/gate-serial.txt"

# Internal mode: run one test, print `name<TAB>seconds<TAB>status` on stdout.
if [[ ${1:-} == --one ]]; then
    bin=$2 logdir=$3 limit=$4 name=$5
    log="$logdir/$(printf '%s' "$name" | tr -c 'A-Za-z0-9_' '_').log"
    start=$EPOCHREALTIME
    timeout --kill-after=10 "$limit" "$bin" --ignored --exact "$name" --nocapture >"$log" 2>&1
    code=$?
    status=ok
    if [[ $code -ne 0 ]]; then
        status=FAIL
        # 124: timeout stopped it; 137: it had to be killed
        [[ $code -eq 124 || $code -eq 137 ]] &&
            echo "gate.sh: timed out after ${limit}s (exit $code)" >>"$log"
    fi
    awk -v n="$name" -v a="$start" -v b="$EPOCHREALTIME" -v s="$status" \
        'BEGIN { printf "%s\t%.1f\t%s\n", n, b - a, s }'
    exit 0
fi

jobs=4
filter=""
limit=300
while getopts "j:f:t:" opt; do
    case $opt in
        j) jobs=$OPTARG ;;
        f) filter=$OPTARG ;;
        t) limit=$OPTARG ;;
        *) echo "usage: scripts/gate.sh [-j N] [-f FILTER] [-t SECS]" >&2; exit 2 ;;
    esac
done
if ! [[ $jobs =~ ^[1-9][0-9]*$ ]]; then
    echo "gate.sh: -j needs a positive number" >&2
    exit 2
fi
if ! [[ $limit =~ ^[1-9][0-9]*$ ]]; then
    echo "gate.sh: -t needs a positive number of seconds" >&2
    exit 2
fi
if [[ -z ${AUTO_POB_POB_DIR:-} ]]; then
    echo "gate.sh: set AUTO_POB_POB_DIR to the pinned PoB folder" >&2
    exit 2
fi
# GitHub sets CI=true, which makes upstream's HeadlessWrapper skip ModCache and changes the
# golden values.
unset CI
export AUTO_POB_POB_DIR

cd "$root" || exit 2
build=$(cargo test -p auto-pob --features gate-probes --test gate --no-run 2>&1)
code=$?
if [[ $code -ne 0 ]]; then
    printf '%s\n' "$build" >&2
    exit "$code"
fi
bin=$(printf '%s\n' "$build" | sed -n 's/^ *Executable tests\/gate\.rs (\(.*\))$/\1/p' | tail -1)
[[ $bin == /* ]] || bin=$root/$bin
if [[ -z $bin || ! -x $bin ]]; then
    echo "gate.sh: couldn't find the built gate binary" >&2
    exit 1
fi

# every ignored test, narrowed by the filter (a substring, as for cargo test)
all=$("$bin" --ignored --list 2>/dev/null | sed -n 's/: test$//p')
selected=$(printf '%s\n' "$all" | grep -F -- "$filter" | grep -v '^$' || true)
if [[ -z $selected ]]; then
    echo "gate.sh: no tests matched '${filter}'" >&2
    exit 1
fi

serial=""
if [[ -f $serial_file ]]; then
    serial=$(sed 's/#.*//; s/^[[:space:]]*//; s/[[:space:]]*$//' "$serial_file" | grep -v '^$' || true)
fi
if [[ -z $filter ]]; then
    while IFS= read -r entry; do
        [[ -z $entry ]] && continue
        printf '%s\n' "$all" | grep -qFx -- "$entry" ||
            echo "gate.sh: warning: gate-serial.txt names '$entry', which is not a gate test" >&2
    done <<<"$serial"
fi
is_serial=$(printf '%s\n' "$selected" | grep -Fx -f <(printf '%s\n' "$serial" | grep -v '^$' || true) || true)
parallel=$(printf '%s\n' "$selected" | grep -Fxv -f <(printf '%s\n' "$is_serial" | grep -v '^$' || echo "//none//") | grep -v '^$' || true)

# longest first, from the last run's times, so the slow tests don't end up last
times=$root/target/gate-times.tsv
order() {
    local list=$1
    if [[ -s $times ]]; then
        awk -F'\t' 'NR==FNR { t[$1] = $2; next } { print (($0 in t) ? t[$0] : 1e9) "\t" $0 }' \
            "$times" <(printf '%s\n' "$list") | sort -t$'\t' -k1,1nr | cut -f2
    else
        printf '%s\n' "$list"
    fi
}

logdir=$(mktemp -d "${TMPDIR:-/tmp}/auto-pob-gate.XXXXXX")
results=$logdir/results.tsv
: >"$results"
total=$(printf '%s\n' "$selected" | wc -l)
echo "gate.sh: $total tests, -j$jobs, logs in $logdir"
began=$(date +%s)

run_batch() { # list, jobs
    [[ -z $1 ]] && return
    order "$1" | xargs -d '\n' -P "$2" -n 1 "$self" --one "$bin" "$logdir" "$limit" >>"$results"
}
run_batch "$parallel" "$jobs"
run_batch "$is_serial" 1

wall=$(( $(date +%s) - began ))
mkdir -p "$root/target"
# a filtered run updates its own tests' times and keeps every other test's, so the next full
# run still orders by the whole suite; a full run starts the file over
merged=$logdir/times.tsv
if [[ -n $filter && -s $times ]]; then
    awk -F'\t' 'NR==FNR { seen[$1] = 1; print; next } !($1 in seen)' "$results" "$times" >"$merged"
else
    cp "$results" "$merged"
fi
sort -t$'\t' -k2,2nr "$merged" >"$times"
passed=$(grep -c $'\tok$' "$results" || true)
failed=$(grep -c $'\tFAIL$' "$results" || true)
sum=$(awk -F'\t' '{ t += $2 } END { printf "%.0f", t }' "$results")
echo "slowest:"
sort -t$'\t' -k2,2nr "$results" | head -5 | sed 's/^/  /'
if [[ $failed -ne 0 || $passed -ne $total ]]; then
    echo "FAILED:"
    for name in $(awk -F'\t' '$3 == "FAIL" { print $1 }' "$results"); do
        echo "  $name"
        tail -n 15 "$logdir/$(printf '%s' "$name" | tr -c 'A-Za-z0-9_' '_').log" | sed 's/^/    /'
    done
    [[ $passed -ne $((total - failed)) || $((passed + failed)) -ne $total ]] &&
        echo "gate.sh: only $((passed + failed)) of $total tests reported" >&2
fi
echo "gate.sh: $passed/$total passed, $failed failed, wall ${wall}s, serial-equivalent ${sum}s (times in target/gate-times.tsv)"
if [[ $failed -eq 0 && $passed -eq $total ]]; then
    rm -rf "$logdir"
    exit 0
fi
echo "gate.sh: logs kept in $logdir" >&2
exit 1
