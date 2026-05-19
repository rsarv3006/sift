#!/usr/bin/env bash
# Real-repo smoke benchmark
# Usage: REPO=/path/to/repo SIFT=target/debug/sift ./benches/real-repo.sh
set -euo pipefail

REPO="${REPO:-/tmp/just}"
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SIFT="${SIFT:-$SCRIPT_DIR/target/debug/sift}"

if [ ! -f "$SIFT" ]; then
  echo "Building sift..."
  (cd "$SCRIPT_DIR" && cargo build 2>/dev/null)
fi

SIFT="$(cd "$(dirname "$SIFT")" && pwd)/$(basename "$SIFT")"

echo "# Real-repo benchmark: $(basename "$REPO")"
echo

# Source stats
echo "## Source"
echo
echo "- Files: $(find "$REPO/src" -name '*.rs' | wc -l)"
echo "- Bytes: $(find "$REPO/src" -name '*.rs' -exec cat {} + | wc -c)"

# Index stats
echo
echo "## Index"
echo
$SIFT index "$REPO" 2>&1 | tail -3

echo
echo "## Queries"
echo
echo '| Query | Results | Sift out (bytes) | Naive cost (bytes) | Savings |'
echo '|-------|---------|------------------|--------------------|---------|'

TOTAL_SIFT=0
TOTAL_NAIVE=0

run_query() {
  local query="$1"
  local symbol="$2"
  local out
  out=$($SIFT query "$query" 2>/dev/null)
  local sift_bytes
  sift_bytes=$(echo "$out" | wc -c)
  
  # Naive: grep scans all src/ files + reads matched files
  local grep_bytes
  grep_bytes=$(find "$REPO/src" -name '*.rs' -exec cat {} + | wc -c)
  local matched_bytes=0
  while IFS= read -r -d '' f; do
    if grep -q "$symbol" "$f" 2>/dev/null; then
      matched_bytes=$((matched_bytes + $(wc -c < "$f")))
    fi
  done < <(find "$REPO/src" -name '*.rs' -print0)
  local naive=$((grep_bytes + matched_bytes))
  
  TOTAL_SIFT=$((TOTAL_SIFT + sift_bytes))
  TOTAL_NAIVE=$((TOTAL_NAIVE + naive))
  
  local result_count
  result_count=$(echo "$out" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
  local savings
  savings=$(echo "scale=0; $naive / $sift_bytes" | bc 2>/dev/null || echo "?")
  
  printf "| %s | %s | %s | %s | %sx |\n" "$query" "$result_count" "$sift_bytes" "$naive" "$savings"
}

cd "$REPO"

run_query "define Config" Config
run_query "calls run" run
run_query "callees run" run
run_query "define Error" Error
run_query "implements Config" Config
run_query "file src/lib.rs" lib
run_query "file src/run.rs" run
run_query "symbols matching config" config

echo
echo "## Summary"
echo
TOTAL_SAVINGS=$(echo "scale=0; $TOTAL_NAIVE / $TOTAL_SIFT" | bc 2>/dev/null || echo "?")
echo "- Total sift output: $TOTAL_SIFT bytes"
echo "- Total naive cost: $TOTAL_NAIVE bytes"
echo "- Avg savings: ${TOTAL_SAVINGS}x"
