#!/bin/bash
# Automated leftwm test harness: Xvfb (real headless display) + leftwm (worker
# + lefthk, via the top-level `leftwm` binary) + leftwm-command/leftwm-state
# (drive/inspect the running instance) + xdotool/wmctrl/import (drive keys,
# list windows, screenshot). Modeled on wm/xmonad/test/harness.sh and
# wm/xmonad/expanse/harness.sh in the parent dotfiles repo — same Xvfb+xscope
# pattern, same "xscope is opt-in" lesson learned there (it busy-spins at
# ~100% CPU forever if its peer dies without a clean `stop`, so it's never
# started unless XSCOPE=1 is set).
#
# leftwm-command/leftwm-state locate their pipe/socket via $DISPLAY (pipe is
# literally named `command-<DISPLAY>.pipe`) under $XDG_RUNTIME_DIR/leftwm/, so
# this harness gives each run an isolated XDG_RUNTIME_DIR — never touches a
# real logged-in leftwm session's pipe/socket even if one happens to be
# running on this machine.
#
# Typical flow:
#   harness.sh start
#   harness.sh spawn xterm
#   harness.sh cmd "GoToTag 2"
#   harness.sh state
#   harness.sh key super+shift+q
#   harness.sh shot /tmp/after.png
#   harness.sh stop

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$HERE/target/release"

STATE_DIR="${LEFTWM_HARNESS_DIR:-/tmp/leftwm-harness}"
STATE_FILE="$STATE_DIR/state.env"
LOGFILE="$STATE_DIR/xscope.log"
RUNTIME_DIR="$STATE_DIR/runtime"

usage() {
    cat <<'EOF'
Usage: harness.sh <command> [args]

  start [WIDTHxHEIGHT]   Start Xvfb + leftwm (default 1600x1080).
                         Set XSCOPE=1 to also proxy through xscope for
                         request accounting (mark/events/log) — opt-in
                         because a leaked xscope busy-spins at ~100% CPU.
  stop                   Kill leftwm/lefthk/xscope/Xvfb, clean up state
  status                 Show whether the harness is running
  key KEYCOMBO           Send a key combo via xdotool (e.g. super+j)
  cmd COMMAND             Send a command via leftwm-command (e.g. "GoToTag 2")
  state                  One-shot raw leftwm-state JSON dump (leftwm-state -q)
  spawn CMD...           Launch CMD against the harness display
  shot PATH              Screenshot root window to PATH (import)
  windows                List windows (wmctrl -l)
  mark LABEL             Record current xscope log position as LABEL (needs XSCOPE=1)
  events LABEL           Count Create/Destroy/Map/UnmapWindow since LABEL (needs XSCOPE=1)
  log [N]                Tail last N lines of the xscope log (default 40)
EOF
    exit 1
}

[ $# -ge 1 ] || usage
cmd="$1"; shift
mkdir -p "$STATE_DIR" "$RUNTIME_DIR"
# xdg-rs enforces 0700 on the runtime dir (XDG base dir spec) — mkdir -p
# leaves it 0755, which leftwm-command/leftwm-state reject as insecure.
chmod 700 "$RUNTIME_DIR"

load_state() { if [ -f "$STATE_FILE" ]; then source "$STATE_FILE"; fi; }
save_state() { : > "$STATE_FILE"; for v in "$@"; do echo "$v" >> "$STATE_FILE"; done; }

find_free_display() {
    for n in $(seq 50 99); do
        if [ ! -e "/tmp/.X${n}-lock" ]; then
            echo "$n"
            return
        fi
    done
    echo "no free display number found" >&2
    exit 1
}

case "$cmd" in
    start)
        load_state
        if [ -n "${LEFTWM_PID:-}" ] && kill -0 "$LEFTWM_PID" 2>/dev/null; then
            echo "already running: real=:$REAL_DNUM leftwm pid=$LEFTWM_PID"
            exit 0
        fi

        [ -x "$BIN_DIR/leftwm" ] || {
            echo "leftwm not built — run: cd $HERE && cargo build --release" >&2
            exit 1
        }

        RES="${1:-1600x1080}"

        REAL_DNUM=$(find_free_display)
        Xvfb ":$REAL_DNUM" -screen 0 "${RES}x24" -listen tcp >"$STATE_DIR/xvfb.log" 2>&1 &
        XVFB_PID=$!
        sleep 0.5
        xdpyinfo -display ":$REAL_DNUM" >/dev/null 2>&1 || {
            echo "Xvfb failed to start, see $STATE_DIR/xvfb.log" >&2
            exit 1
        }

        WM_DNUM="$REAL_DNUM"
        XSCOPE_PID=""; PROXY_DNUM=""
        if [ "${XSCOPE:-0}" = 1 ]; then
            PROXY_DNUM=$(find_free_display)
            : > "$LOGFILE"
            xscope -i"$PROXY_DNUM" -o"$REAL_DNUM" -h127.0.0.1 -v1 >"$LOGFILE" 2>&1 &
            XSCOPE_PID=$!
            sleep 0.5
            WM_DNUM="$PROXY_DNUM"
        fi

        # setsid puts leftwm (and the leftwm-worker/lefthk-worker children it
        # forks) in their own process group, so `stop` can kill the whole
        # group at once instead of hunting down each child pid individually.
        setsid env \
            DISPLAY=":$WM_DNUM" \
            XDG_RUNTIME_DIR="$RUNTIME_DIR" \
            PATH="$BIN_DIR:$PATH" \
            leftwm >"$STATE_DIR/leftwm.out" 2>"$STATE_DIR/leftwm.err" &
        LEFTWM_PID=$!
        sleep 1
        if ! kill -0 "$LEFTWM_PID" 2>/dev/null; then
            echo "leftwm failed to start, see $STATE_DIR/leftwm.err" >&2
            cat "$STATE_DIR/leftwm.err" >&2
            exit 1
        fi

        save_state "XVFB_PID=$XVFB_PID" "REAL_DNUM=$REAL_DNUM" \
                    "XSCOPE_PID=$XSCOPE_PID" "PROXY_DNUM=$PROXY_DNUM" \
                    "LEFTWM_PID=$LEFTWM_PID" "WM_DNUM=$WM_DNUM"
        echo "started: real display=:$REAL_DNUM (Xvfb pid $XVFB_PID)"
        echo "         leftwm display=:$WM_DNUM${XSCOPE_PID:+ (via xscope pid $XSCOPE_PID)}, leftwm pid $LEFTWM_PID"
        echo "         runtime dir: $RUNTIME_DIR"
        if [ -n "$XSCOPE_PID" ]; then echo "         xscope log: $LOGFILE"; fi
        ;;

    stop)
        load_state
        if [ -n "${LEFTWM_PID:-}" ]; then
            kill -- "-$LEFTWM_PID" 2>/dev/null || true
        fi
        for pid in "${XSCOPE_PID:-}" "${XVFB_PID:-}"; do
            [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
        done
        # Safety sweep, same lesson as expanse/harness.sh: a leaked xscope
        # never self-exits and busy-spins at ~100% CPU, so reap by signature
        # even if the state file was lost.
        pkill -f 'xscope .*-h127.0.0.1 -v1' 2>/dev/null || true
        rm -f "$STATE_FILE"
        rm -rf "$RUNTIME_DIR"
        echo "stopped"
        ;;

    status)
        load_state
        if [ -n "${LEFTWM_PID:-}" ] && kill -0 "$LEFTWM_PID" 2>/dev/null; then
            echo "running: real=:$REAL_DNUM leftwm pid=$LEFTWM_PID"
        else
            echo "not running"
        fi
        ;;

    key)
        load_state
        [ $# -ge 1 ] || { echo "usage: harness.sh key KEYCOMBO" >&2; exit 1; }
        DISPLAY=":$REAL_DNUM" xdotool key "$1"
        ;;

    cmd)
        load_state
        [ $# -ge 1 ] || { echo "usage: harness.sh cmd COMMAND" >&2; exit 1; }
        DISPLAY=":$WM_DNUM" XDG_RUNTIME_DIR="$RUNTIME_DIR" PATH="$BIN_DIR:$PATH" \
            leftwm-command "$1"
        ;;

    state)
        load_state
        DISPLAY=":$WM_DNUM" XDG_RUNTIME_DIR="$RUNTIME_DIR" PATH="$BIN_DIR:$PATH" \
            leftwm-state -q
        ;;

    spawn)
        load_state
        [ $# -ge 1 ] || { echo "usage: harness.sh spawn CMD..." >&2; exit 1; }
        DISPLAY=":$REAL_DNUM" "$@" &
        echo "spawned pid $!"
        ;;

    shot)
        load_state
        [ $# -ge 1 ] || { echo "usage: harness.sh shot PATH" >&2; exit 1; }
        DISPLAY=":$REAL_DNUM" import -window root "$1"
        echo "saved $1"
        ;;

    windows)
        load_state
        DISPLAY=":$REAL_DNUM" wmctrl -l
        ;;

    mark)
        load_state
        [ -f "$LOGFILE" ] || { echo "mark/events/log need xscope — restart with XSCOPE=1" >&2; exit 1; }
        [ $# -ge 1 ] || { echo "usage: harness.sh mark LABEL" >&2; exit 1; }
        wc -l < "$LOGFILE" > "$STATE_DIR/mark-$1"
        echo "marked '$1' at $(cat "$STATE_DIR/mark-$1") lines"
        ;;

    events)
        load_state
        [ -f "$LOGFILE" ] || { echo "mark/events/log need xscope — restart with XSCOPE=1" >&2; exit 1; }
        [ $# -ge 1 ] || { echo "usage: harness.sh events LABEL" >&2; exit 1; }
        MARK_FILE="$STATE_DIR/mark-$1"
        [ -f "$MARK_FILE" ] || { echo "no mark '$1' -- run 'mark $1' first" >&2; exit 1; }
        START_LINE=$(cat "$MARK_FILE")
        TOTAL_LINES=$(wc -l < "$LOGFILE")
        SLICE=$(tail -n +"$((START_LINE + 1))" "$LOGFILE")
        echo "--- leftwm requests since mark '$1' ($((TOTAL_LINES - START_LINE)) lines) ---"
        for op in CreateWindow DestroyWindow MapWindow UnmapWindow ReparentWindow ConfigureWindow; do
            n=$(grep -c "$op" <<<"$SLICE" || true)
            echo "$op: $n"
        done
        ;;

    log)
        N="${1:-40}"
        tail -n "$N" "$LOGFILE"
        ;;

    *)
        usage
        ;;
esac
