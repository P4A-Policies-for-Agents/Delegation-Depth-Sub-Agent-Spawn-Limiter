#!/usr/bin/env bash
# Live demo: run the multi-agent orchestrator against the governed gateway and
# watch the Delegation Depth & Sub-Agent Spawn Limiter cut the chain / fan-out.
set -uo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
[ -f "$DIR/env.local.sh" ] && . "$DIR/env.local.sh"
: "${DD_GW_URL:?Set DD_GW_URL (see demo/env.local.sh.example)}"
python3 "$DIR/agent.py"
