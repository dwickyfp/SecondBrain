#!/usr/bin/env bash
set -euo pipefail

artifact="${1:?usage: smoke-installer.sh <exact .dmg or .deb> <marker> <memory-json> <commit> <fixture-sha256>}"
marker="${2:?missing readiness marker path}"
memory="${3:?missing memory receipt path}"
commit="${4:?missing full commit}"
fixture_sha="${5:?missing fixture SHA-256}"
rm -f "$marker" "$memory"

case "$(uname -s)" in
  Darwin)
    mount_dir="$(mktemp -d)"
    cleanup_macos() {
      test -z "${pid:-}" || kill "$pid" >/dev/null 2>&1 || true
      test -z "${pid:-}" || wait "$pid" 2>/dev/null || true
      hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
      rmdir "$mount_dir" >/dev/null 2>&1 || true
    }
    trap cleanup_macos EXIT
    hdiutil attach -readonly -nobrowse -mountpoint "$mount_dir" "$artifact"
    executable="$mount_dir/SecondBrain.app/Contents/MacOS/secondbrain-desktop"
    test -x "$executable"
    SB_READINESS_MARKER="$marker" "$executable" &
    pid=$!
    ;;
  Linux)
    extract_dir="$(mktemp -d)"
    display_number=99
    while test -e "/tmp/.X11-unix/X$display_number"; do display_number=$((display_number + 1)); done
    Xvfb ":$display_number" -screen 0 1280x720x24 >/dev/null 2>&1 &
    xvfb_pid=$!
    trap 'test -z "${pid:-}" || kill "$pid" >/dev/null 2>&1 || true; test -z "${pid:-}" || wait "$pid" 2>/dev/null || true; kill "$xvfb_pid" >/dev/null 2>&1 || true; wait "$xvfb_pid" 2>/dev/null || true; rm -rf "$extract_dir"' EXIT
    dpkg-deb --extract "$artifact" "$extract_dir"
    executable="$(command find "$extract_dir/usr/bin" -maxdepth 1 -type f -perm -111 -print -quit)"
    test -n "$executable"
    DISPLAY=":$display_number" SB_READINESS_MARKER="$marker" "$executable" &
    pid=$!
    ;;
  *)
    echo "unsupported smoke platform: $(uname -s)" >&2
    exit 2
    ;;
esac

for _ in $(seq 1 60); do
  test -f "$marker" && break
  kill -0 "$pid" 2>/dev/null || { wait "$pid"; exit 1; }
  sleep 1
done
test -f "$marker"
node "$(dirname "$0")/verify-readiness.mjs" "$marker" "$commit" "$fixture_sha"
sleep 5
node "$(dirname "$0")/sample-process-tree-rss.mjs" "$pid" "$memory" 500 20
test -s "$memory"
kill -0 "$pid" 2>/dev/null
kill "$pid" >/dev/null 2>&1 || true
wait "$pid" 2>/dev/null || true
