#!/usr/bin/env bash
# tools/check.sh - quiet build + test of just the packages you name (Git Bash).
#   tools/check.sh -p <crate> [-p <crate> ...] [--release]
# <crate> is the package name (bu-audio, pane_test_a ...).
# Prints ONLY: compiler errors/warnings (deduplicated), failing tests with their panic line, and a last line
# "CHECK PASS" or "CHECK FAIL (<n> errors, <m> failed tests)"; on FAIL also the last 30 raw lines.
# The full raw log is target/check.log.
set -u
cd "$(dirname "$0")/.." || exit 2
export CARGO_TERM_COLOR=never

pkgs=(); rel=()
while [ $# -gt 0 ]; do
  case "$1" in
    -p|--package) [ $# -ge 2 ] || { echo "check.sh: -p needs a crate name"; exit 2; }; pkgs+=(-p "$2"); shift 2 ;;
    --release) rel=(--release); shift ;;
    *) echo "usage: tools/check.sh -p <crate> [-p <crate>...] [--release]"; exit 2 ;;
  esac
done
[ ${#pkgs[@]} -gt 0 ] || { echo "usage: tools/check.sh -p <crate> [-p <crate>...] [--release]"; exit 2; }

mkdir -p target
log=target/check.log
: > "$log"

run() { # run <cargo args...>; appends to the log, returns cargo's exit code
  echo "===== cargo $* =====" >> "$log"
  cargo "$@" >> "$log" 2>&1
  local rc=$?
  echo "===== exit $rc =====" >> "$log"
  return $rc
}

run build "${pkgs[@]}" "${rel[@]}"; build_rc=$?
test_rc=0
if [ $build_rc -eq 0 ]; then run test "${pkgs[@]}" "${rel[@]}"; test_rc=$?; fi

# compiler diagnostics: header line + its "--> file:line" line, deduplicated, cargo's summary lines dropped
diag=$(awk '
  /^(error|warning)(\[[A-Za-z0-9]+\])?: / {
    if ($0 ~ /^warning: `.*` \(.*\) generated [0-9]+ warning/) { hdr=""; next }
    if ($0 ~ /^error: (could not compile|aborting due to|test failed|build failed)/) { hdr=""; next }
    if ($0 ~ /^error: [0-9]+ test/) { hdr=""; next }
    hdr=$0; next
  }
  /^ *--> / { if (hdr != "") { sub(/^ *--> */, "", $0); print hdr "  [" $0 "]"; hdr="" } next }
' "$log" | awk '!seen[$0]++')
n_err=0
[ -n "$diag" ] && n_err=$(printf '%s\n' "$diag" | grep -c '^error')

# failing tests: name + the panic line that follows its "---- name stdout ----" block
fails=$(awk '
  /^test .* \.\.\. FAILED$/ { n=$2; if (!(n in isfail)) { isfail[n]=1; order[++c]=n } }
  /^---- .* stdout ----$/ { cur=$2; next }
  /^thread .* panicked at / { if (cur != "" && !(cur in pan)) { p=$0; sub(/^thread .* panicked at /, "", p); sub(/:$/, "", p); pan[cur]=p; want=cur } next }
  want != "" { pan[want]=pan[want] " - " $0; want="" }
  END { for (i=1;i<=c;i++) { n=order[i]; print n "  " ((n in pan) ? pan[n] : "(no panic line)") } }
' "$log")
n_fail=0
[ -n "$fails" ] && n_fail=$(printf '%s\n' "$fails" | grep -c .)

[ -n "$diag" ] && printf '%s\n' "$diag"
[ -n "$fails" ] && { echo "failed tests:"; printf '%s\n' "$fails" | sed 's/^/  /'; }

if [ $build_rc -eq 0 ] && [ $test_rc -eq 0 ]; then
  echo "CHECK PASS"
  exit 0
fi
# a failure with no error line of its own (linker, cargo, a crashed test binary) still counts as one error
if [ "$n_err" -eq 0 ] && [ "$n_fail" -eq 0 ]; then n_err=1; fi
echo "--- last 30 raw lines (full log: target/check.log) ---"
tail -n 30 "$log"
echo "CHECK FAIL ($n_err errors, $n_fail failed tests)"
exit 1
