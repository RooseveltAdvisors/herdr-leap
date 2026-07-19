#!/usr/bin/env bash
# Deterministic guarded-lab proof for herdr-leap smart-nav actions.
# Use only with fm-herdr-lab.sh and a named non-default session.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="${HERDR_LAB_HELPER:-/opt/ra/firstmate/bin/fm-herdr-lab.sh}"
if [[ ! -x "$HELPER" ]]; then
  echo "lab-smart-nav: HERDR_LAB_HELPER not executable: $HELPER" >&2
  exit 2
fi

SESSION="${HERDR_LAB_SESSION:-}"
OWNED_SESSION=0
if [[ -z "$SESSION" ]]; then
  SESSION="$("$HELPER" name herdr-smart-pane-navigator)"
  OWNED_SESSION=1
fi

H() { "$HELPER" run "$SESSION" "$@"; }

cleanup() {
  local code=$?
  if [[ "$OWNED_SESSION" -eq 1 ]]; then
    "$HELPER" teardown "$SESSION" >/dev/null 2>&1 || true
  fi
  exit "$code"
}
trap cleanup EXIT

if [[ "$OWNED_SESSION" -eq 1 ]]; then
  "$HELPER" provision "$SESSION" >/dev/null
fi

echo "lab-smart-nav: session=$SESSION"

# Build release binary for the plugin actions.
(cd "$ROOT" && cargo build --release --locked) >/tmp/lab-smart-nav-build.log 2>&1

H plugin link "$ROOT" >/dev/null

# Fresh 2x2 fixture in an isolated workspace.
H workspace create --label lab-smart-nav --cwd /tmp --focus >/dev/null
H pane split --direction right >/dev/null
H pane focus --direction left >/dev/null
H pane split --direction down >/dev/null
H pane focus --direction right >/dev/null
H pane split --direction down >/dev/null

LAYOUT=$(H pane layout)
WS=$(echo "$LAYOUT" | jq -r '.result.layout.workspace_id')
mapfile -t PANES < <(echo "$LAYOUT" | jq -r '
  .result.layout.panes
  | sort_by(.rect.y, .rect.x)
  | .[].pane_id
')
if [[ "${#PANES[@]}" -lt 4 ]]; then
  echo "lab-smart-nav: expected 4 panes, got ${#PANES[@]}: ${PANES[*]-}" >&2
  exit 1
fi
P1=${PANES[0]}
P2=${PANES[1]}
P3=${PANES[2]}
P4=${PANES[3]}
echo "lab-smart-nav: panes p1=$P1 p2=$P2 p3=$P3 p4=$P4 ws=$WS"

focused() {
  H pane list | jq -r --arg ws "$WS" '
    [.result.panes[] | select(.focused == true and .workspace_id == $ws) | .pane_id][0] // empty
  '
}

force_focus() {
  local target=$1
  case "$target" in
    "$P1") H pane focus --direction left --pane "$P2" >/dev/null || true
           H pane focus --direction up --pane "$P3" >/dev/null || true ;;
    "$P2") H pane focus --direction right --pane "$P1" >/dev/null ;;
    "$P3") H pane focus --direction down --pane "$P1" >/dev/null ;;
    "$P4") H pane focus --direction down --pane "$P2" >/dev/null ;;
  esac
  sleep 0.05
}

LAST_LOG_ID=""

plugin_logs() {
  H plugin log list --plugin RooseveltAdvisors.herdr-leap --limit 50
}

last_log_stdout() {
  plugin_logs | jq -r --arg log_id "$LAST_LOG_ID" '
    [.result.logs[]? | select(.log_id == $log_id)][0].stdout // empty
  ' | tr -d '\r' | sed 's/[[:space:]]*$//'
}

last_log_status() {
  plugin_logs | jq -r --arg log_id "$LAST_LOG_ID" '
    [.result.logs[]? | select(.log_id == $log_id)][0].status // "missing"
  '
}

last_log_details() {
  plugin_logs | jq -r --arg log_id "$LAST_LOG_ID" '
    [.result.logs[]? | select(.log_id == $log_id)][0] as $log
    | if $log == null then "log missing"
      else "status=\($log.status) error=\($log.error // "") stderr=\($log.stderr // "") stdout=\($log.stdout // "")"
      end
  ' | tr -d '\r'
}

invoke() {
  local action=$1 response st i
  if ! response=$(H plugin action invoke "RooseveltAdvisors.herdr-leap.${action}"); then
    echo "FAIL $action: plugin action invocation failed" >&2
    return 1
  fi
  if ! LAST_LOG_ID=$(jq -er '.result.log.log_id | strings | select(length > 0)' <<<"$response"); then
    echo "FAIL $action: invocation returned no log id: $response" >&2
    return 1
  fi

  for ((i = 0; i < 100; i++)); do
    st=$(last_log_status)
    case "$st" in
      succeeded) return 0 ;;
      running|missing) sleep 0.05 ;;
      failed)
        echo "FAIL $action: $(last_log_details)" >&2
        return 1
        ;;
      *)
        echo "FAIL $action: unexpected log status '$st'" >&2
        return 1
        ;;
    esac
  done

  echo "FAIL $action: timed out waiting for log $LAST_LOG_ID: $(last_log_details)" >&2
  return 1
}

assert_eq() {
  local label=$1 got=$2 want=$3
  if [[ "$got" != "$want" ]]; then
    echo "FAIL $label: got=$got want=$want log=$(last_log_stdout)" >&2
    exit 1
  fi
  echo "PASS $label ($got)"
}

fg_name() {
  H pane process-info --pane "$1" \
    | jq -r '.result.process_info.foreground_processes[0].name // empty'
}

wait_fg() {
  local pane=$1 want=$2
  local i name
  for ((i = 0; i < 40; i++)); do
    name=$(fg_name "$pane")
    if [[ "$name" == "$want" ]]; then
      echo "$name"
      return 0
    fi
    sleep 0.1
  done
  echo "$name"
  return 1
}

# --- A. Shell four-direction circuit ---
force_focus "$P1"
assert_eq "start p1" "$(focused)" "$P1"
invoke smart-right
assert_eq "shell right" "$(focused)" "$P2"
invoke smart-down
assert_eq "shell down" "$(focused)" "$P4"
invoke smart-left
assert_eq "shell left" "$(focused)" "$P3"
invoke smart-up
assert_eq "shell up" "$(focused)" "$P1"

# --- B. No-neighbor edges ---
force_focus "$P1"
invoke smart-left
assert_eq "no-neighbor left" "$(focused)" "$P1"
invoke smart-up
assert_eq "no-neighbor up" "$(focused)" "$P1"

# --- C. Nvim passthrough ---
force_focus "$P2"
# Clear any half-typed input, then start nvim.
H pane send-keys "$P2" ctrl+c >/dev/null || true
sleep 0.1
H pane send-text "$P2" "nvim -u NONE -n --cmd 'set noswapfile' /tmp/lab-smart-nav.txt"
H pane send-keys "$P2" enter >/dev/null
if ! name=$(wait_fg "$P2" nvim); then
  echo "FAIL nvim process: got=${name:-empty} want=nvim" >&2
  H pane read --pane "$P2" --source visible --format text 2>/dev/null | tail -5 >&2 || true
  exit 1
fi
assert_eq "nvim process" "$name" "nvim"
force_focus "$P2"
invoke smart-left
assert_eq "nvim stay left" "$(focused)" "$P2"
log=$(last_log_stdout)
if ! grep -q 'passthrough' <<<"$log"; then
  echo "FAIL nvim passthrough log: $log" >&2
  exit 1
fi
echo "PASS nvim passthrough log ($log)"

# Exit nvim
H pane send-keys "$P2" escape escape >/dev/null
H pane send-keys "$P2" ':' >/dev/null
H pane send-text "$P2" 'qa!'
H pane send-keys "$P2" enter >/dev/null
sleep 0.4

# --- D. Cross-workspace isolation ---
H workspace create --label other-ws --cwd /tmp >/dev/null
# Return focus to lab workspace via a pane in it.
force_focus "$P2"
invoke smart-right
assert_eq "edge right no cross-ws" "$(focused)" "$P2"
# Confirm other workspace root never became focused.
other_focused=$(H pane list | jq -r '
  [.result.panes[] | select(.focused == true) | .workspace_id][0] // empty
')
assert_eq "still on lab workspace" "$other_focused" "$WS"

echo "lab-smart-nav: ALL PASS"
