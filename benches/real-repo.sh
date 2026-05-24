#!/usr/bin/env bash
# Real-repo benchmark — language-agnostic.
# Usage: REPO=/path/to/repo ./benches/real-repo.sh
#   Optionally: QUERIES="define init|callers write|symbols matching lock"
set -euo pipefail

REPO="${REPO:-/tmp/just}"
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SIFT="${SIFT:-$SCRIPT_DIR/target/release/sift}"

if [ ! -f "$SIFT" ]; then
  echo "Building sift (release)..."
  (cd "$SCRIPT_DIR" && cargo build --release 2>/dev/null)
fi
SIFT="$(cd "$(dirname "$SIFT")" && pwd)/$(basename "$SIFT")"

# All source extensions sift understands (from parser.rs LanguageId::from_path)
EXTENSIONS=(rs py js jsx ts tsx go c h cpp cxx cc hpp hh hxx java rb zig sh bash)

# Build find -name args for source extensions
EXT_FIND=()
for ext in "${EXTENSIONS[@]}"; do
  EXT_FIND+=(-o -name "*.$ext")
done

count_files() {
  find "$1" -type f \( -false "${EXT_FIND[@]}" \) -print0 2>/dev/null | tr -d -c '\0' | wc -c
}

total_bytes() {
  find "$1" -type f \( -false "${EXT_FIND[@]}" \) -exec cat {} + 2>/dev/null | wc -c
}

echo "# Real-repo benchmark: $(basename "$REPO")"
echo

# Source stats
echo "## Source"
echo
echo "- Files: $(count_files "$REPO")"
echo "- Bytes: $(total_bytes "$REPO")"

# Index
echo
echo "## Index"
echo
$SIFT index "$REPO" 2>&1 | tail -5

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

  # Naive: grep scans all source files + reads matched files
  local grep_bytes matched_bytes=0
  grep_bytes=$(total_bytes "$REPO")

  local tmpf
  tmpf=$(mktemp)
  find "$REPO" -type f \( -false "${EXT_FIND[@]}" \) -print0 2>/dev/null > "$tmpf"
  while IFS= read -r -d '' f; do
    if grep -q "$symbol" "$f" 2>/dev/null; then
      matched_bytes=$((matched_bytes + $(wc -c < "$f")))
    fi
  done < "$tmpf"
  rm -f "$tmpf"

  local naive=$((grep_bytes + matched_bytes))

  TOTAL_SIFT=$((TOTAL_SIFT + sift_bytes))
  TOTAL_NAIVE=$((TOTAL_NAIVE + naive))

  local result_count
  if echo "$out" | grep -q "^No results"; then
    result_count=0
  else
    result_count=$(echo "$out" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "err")
  fi
  local savings
  savings=$(echo "scale=0; $naive / $sift_bytes" | bc 2>/dev/null || echo "?")

  printf "| %s | %s | %s | %s | %sx |\n" "$query" "$result_count" "$sift_bytes" "$naive" "$savings"
}

cd "$REPO"

# Default queries: symbols common across C/Rust/Go/… codebases.
# Override with QUERIES="define init:init|callers write:write" for repo-specific queries.
DEFAULT_QUERIES=(
  "define init:init"
  "define read:read"
  "define write:write"
  "define new:new"
  "symbols matching config:config"
  "symbols matching error:error"
)

if [ -n "${QUERIES:-}" ]; then
  IFS='|' read -ra QS <<< "$QUERIES"
  for qe in "${QS[@]}"; do
    IFS=':' read -r qry sym <<< "$qe"
    run_query "$qry" "$sym"
  done
else
  for qe in "${DEFAULT_QUERIES[@]}"; do
    IFS=':' read -r qry sym <<< "$qe"
    run_query "$qry" "$sym"
  done
fi

echo
echo "## Summary"
echo
TOTAL_SAVINGS=$(echo "scale=0; $TOTAL_NAIVE / $TOTAL_SIFT" | bc 2>/dev/null || echo "?")
echo "- Total sift output: $TOTAL_SIFT bytes"
echo "- Total naive cost: $TOTAL_NAIVE bytes"
echo "- Avg savings: ${TOTAL_SAVINGS}x"
