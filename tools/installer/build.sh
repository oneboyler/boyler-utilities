#!/usr/bin/env bash
# tools/installer/build.sh (Order 032) - builds `dist/Boyler Utilities Setup.exe` (Inno Setup 6, per user, no admin).
#   tools/installer/build.sh                 cargo build --release -p bu-app, then the setup
#   tools/installer/build.sh --exe <path>    package an exe that is already built (no cargo)
#   tools/installer/build.sh --out <dir>     write the setup there instead of <repo>/dist
#   tools/installer/build.sh --define N=V    extra ISCC define (proof builds only: ProofNoMsi=1)
# Inno Setup: %LOCALAPPDATA%\Programs\Inno Setup 6 (installed per user: innosetup-6.7.3.exe /CURRENTUSER /VERYSILENT).
# The version is read from app/Cargo.toml. Everything for Search is set up by the app itself (--install-everything, Order 049:
# the same code as the Search tab's button), so the setup and the Search tab can never disagree.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
exe=""
out="$root/dist"
extra=()
while [ $# -gt 0 ]; do
  case "$1" in
    --exe) exe="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --define) extra+=("/D$2"); shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

iscc="${LOCALAPPDATA:-$HOME/AppData/Local}/Programs/Inno Setup 6/ISCC.exe"
[ -f "$iscc" ] || { echo "Inno Setup not found: $iscc" >&2; exit 1; }

if [ -z "$exe" ]; then
  # the builder's folders never go into the exe (panic messages carry source paths): the cargo home and the repo are
  # rewritten to neutral names (Order 044, open source); its own target folder keeps these flags away from dev builds
  cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$(cygpath -w "$cargo_home")=cargo --remap-path-prefix=$(cygpath -w "$root")=boyler-utilities"
  export CARGO_TARGET_DIR="$root/target/dist"
  (cd "$root" && cargo build --release -p bu-app)
  exe="$root/target/dist/release/BoylerUtilities.exe"
fi
[ -f "$exe" ] || { echo "no exe: $exe" >&2; exit 1; }
exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"   # ISCC resolves a relative path against the .iss folder

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/app/Cargo.toml" | head -1)"
for v in version; do
  [ -n "${!v}" ] || { echo "could not read $v" >&2; exit 1; }
done

mkdir -p "$out"
w() { cygpath -w "$1"; }
# (Git Bash would turn the /D switches into paths)
MSYS2_ARG_CONV_EXCL="*" "$iscc" /Q \
  "/DAppVersion=$version" \
  "/DSrcRoot=$(w "$root")" \
  "/DAppExeSrc=$(w "$exe")" \
  "/DOutDir=$(w "$out")" \
  ${extra[@]+"${extra[@]}"} \
  "$(w "$root/tools/installer/BoylerUtilities.iss")"
setup="$out/Boyler Utilities Setup.exe"
echo "SETUP $(w "$setup") $(stat -c %s "$setup") bytes (version $version)"
