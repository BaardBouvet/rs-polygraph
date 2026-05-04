#!/usr/bin/env bash
# tck_diff.sh — Phase 0 instrumentation for spec-first-pivot.md.
#
# Compare a fresh TCK run against the frozen baseline at
# tests/tck/baseline/scenarios.jsonl. Prints regressions (was-pass, now-fail)
# and improvements (was-fail, now-pass), with exit code 1 on any regression.
#
# Usage:
#   tools/tck_diff.sh                    # run TCK, diff against baseline
#   tools/tck_diff.sh --freeze           # run TCK, overwrite baseline
#   tools/tck_diff.sh --against FILE     # diff arbitrary jsonl against baseline
#
# Requires: jq, cargo, the polygraph TCK harness.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE="${REPO_ROOT}/tests/tck/baseline/scenarios.jsonl"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

mode="diff"
input=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --freeze) mode="freeze"; shift ;;
        --against) input="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,15p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

run_tck() {
    local out="$1"
    cd "${REPO_ROOT}"
    cargo build --tests --quiet
    # Locate the most recent tck binary (cargo's hashed path).
    local bin
    bin="$(ls -t target/debug/deps/tck-* 2>/dev/null | grep -Ev '\.(d|rmeta)$' | head -n 1 || true)"
    if [[ -z "${bin}" ]]; then
        echo "could not locate tck test binary; run 'cargo build --tests' first" >&2
        exit 2
    fi
    POLYGRAPH_TCK_RESULTS_PATH="${out}" "${bin}" >/dev/null 2>&1 || true
}

if [[ "${mode}" == "freeze" ]]; then
    mkdir -p "$(dirname "${BASELINE}")"
    run_tck "${BASELINE}"
    echo "Baseline frozen to ${BASELINE}"
    jq -r '.status' "${BASELINE}" | sort | uniq -c
    exit 0
fi

# diff mode
if [[ -z "${input}" ]]; then
    input="${TMPDIR}/current.jsonl"
    run_tck "${input}"
fi

if [[ ! -f "${BASELINE}" ]]; then
    echo "No baseline at ${BASELINE}. Run with --freeze first." >&2
    exit 2
fi

# Key scenarios by (feature_path, line) so renames don't fool the diff.
key_status() {
    jq -r '[.feature_path, (.line|tostring), .scenario, .status] | @tsv' "$1" \
        | sort
}

key_status "${BASELINE}" > "${TMPDIR}/base.tsv"
key_status "${input}"    > "${TMPDIR}/curr.tsv"

# Build associative arrays via awk (portable; no GNU-only features).
awk -F'\t' '
    NR==FNR { base[$1"|"$2] = $4; base_name[$1"|"$2] = $3; next }
    {
        k = $1"|"$2
        if (!(k in base)) {
            print "ADDED\t" $4 "\t" $1 ":" $2 "\t" $3
            next
        }
        if (base[k] != $4) {
            print "CHANGED\t" base[k] "->" $4 "\t" $1 ":" $2 "\t" $3
        }
        delete base[k]
    }
    END {
        for (k in base) {
            split(k, a, "|")
            print "REMOVED\t" base[k] "\t" a[1] ":" a[2] "\t" base_name[k]
        }
    }
' "${TMPDIR}/base.tsv" "${TMPDIR}/curr.tsv" > "${TMPDIR}/diff.tsv"

regressions="$(awk -F'\t' '$2=="pass->fail" || $2=="pass->skip"' "${TMPDIR}/diff.tsv" || true)"
improvements="$(awk -F'\t' '$2=="fail->pass" || $2=="skip->pass"' "${TMPDIR}/diff.tsv" || true)"
other="$(awk -F'\t' '$1=="ADDED" || $1=="REMOVED"' "${TMPDIR}/diff.tsv" || true)"

base_pass=$(awk -F'\t' '$4=="pass"' "${TMPDIR}/base.tsv" | wc -l | tr -d ' ')
curr_pass=$(awk -F'\t' '$4=="pass"' "${TMPDIR}/curr.tsv" | wc -l | tr -d ' ')
base_total=$(wc -l < "${TMPDIR}/base.tsv" | tr -d ' ')
curr_total=$(wc -l < "${TMPDIR}/curr.tsv" | tr -d ' ')

echo "Baseline: ${base_pass}/${base_total} passing"
echo "Current:  ${curr_pass}/${curr_total} passing"
echo

if [[ -n "${improvements}" ]]; then
    n=$(printf '%s\n' "${improvements}" | wc -l | tr -d ' ')
    echo "Improvements (${n}):"
    printf '%s\n' "${improvements}" | sed 's/^/  /'
    echo
fi

if [[ -n "${other}" ]]; then
    n=$(printf '%s\n' "${other}" | wc -l | tr -d ' ')
    echo "Added/removed scenarios (${n}):"
    printf '%s\n' "${other}" | sed 's/^/  /'
    echo
fi

if [[ -n "${regressions}" ]]; then
    n=$(printf '%s\n' "${regressions}" | wc -l | tr -d ' ')
    echo "REGRESSIONS (${n}):"
    printf '%s\n' "${regressions}" | sed 's/^/  /'
    exit 1
fi

echo "No regressions."
