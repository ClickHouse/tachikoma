#!/usr/bin/env bash
# Start a TCP bridge from the Tart vmnet interface to the host Docker socket.
# Run this on the HOST before spawning a VM with docker-host-forwarding.sh.
#
# Usage:
#   ./examples/docker-bridge-start.sh          # start bridge (foreground)
#   ./examples/docker-bridge-start.sh --bg     # start bridge (background, PID printed)
#   ./examples/docker-bridge-start.sh --stop   # stop background bridge
#
# Requires: socat (brew install socat)
#
# SECURITY WARNING:
#   This exposes the Docker daemon on 192.168.64.1:2375 (no TLS).
#   Bind address is the Tart vmnet bridge — only VMs on that subnet can reach it.
#   Do NOT change to 0.0.0.0 unless you understand the implications.

set -euo pipefail

BIND="${DOCKER_BRIDGE_BIND:-192.168.64.1}"
PORT="${DOCKER_BRIDGE_PORT:-2375}"
PIDFILE="${HOME}/.config/tachikoma/docker-bridge.pid"

# Auto-detect working Docker socket if not explicitly set
detect_socket() {
    if [ -n "${DOCKER_SOCKET:-}" ]; then
        echo "$DOCKER_SOCKET"
        return
    fi
    for sock in \
        "${HOME}/.orbstack/run/docker.sock" \
        "${HOME}/.docker/run/docker.sock" \
        "/var/run/docker.sock"; do
        if [ -S "$sock" ] && curl -sf --unix-socket "$sock" http://localhost/version >/dev/null 2>&1; then
            echo "$sock"
            return
        fi
    done
    echo ""
}

if [ "${1:-}" = "--stop" ]; then
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid"
            echo "Stopped Docker bridge (PID $pid)"
        else
            echo "Bridge not running (stale PID $pid)"
        fi
        rm -f "$PIDFILE"
    else
        echo "No PID file found at $PIDFILE"
    fi
    exit 0
fi

if ! command -v socat &>/dev/null; then
    echo "Error: socat not found. Install with: brew install socat"
    exit 1
fi

SOCKET=$(detect_socket)
if [ -z "$SOCKET" ]; then
    echo "Error: no working Docker socket found."
    echo "Checked: ~/.orbstack/run/docker.sock, ~/.docker/run/docker.sock, /var/run/docker.sock"
    echo "Is Docker Desktop or OrbStack running?"
    exit 1
fi

echo "Bridging ${BIND}:${PORT} → ${SOCKET}"
echo "VMs can reach Docker via DOCKER_HOST=tcp://${BIND}:${PORT}"

if [ "${1:-}" = "--bg" ]; then
    mkdir -p "$(dirname "$PIDFILE")"
    nohup socat "TCP-LISTEN:${PORT},bind=${BIND},reuseaddr,fork" "UNIX-CONNECT:${SOCKET}" \
        > /dev/null 2>&1 &
    echo $! > "$PIDFILE"
    echo "Started in background (PID $!)"
else
    exec socat "TCP-LISTEN:${PORT},bind=${BIND},reuseaddr,fork" "UNIX-CONNECT:${SOCKET}"
fi
