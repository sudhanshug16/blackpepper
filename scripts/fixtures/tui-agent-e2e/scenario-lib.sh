#!/usr/bin/env bash

# Provider scenario shared by the live TUI acceptance entry point.

wait_provider_state() {
  local provider="$1" state="$2" source="$3" kind="$4" active="$5"
  state_tool wait-state "$provider" "$state" "$source" "$kind" "$active" ||
    fail_agent_e2e "$provider did not persist $state from $source"
}

assert_status_evidence() {
  local provider="$1" authority="$2" event="$3" capability="$4" label="$5"
  run_tui_command ':status explain'
  wait_for_screen 'AGENT STATUS EVIDENCE' "$label" 20
  assert_screen_has "$provider" "$label-provider"
  assert_screen_has 'authority' "$label-authority-label"
  assert_screen_has "$authority" "$label-authority"
  assert_screen_has 'last event' "$label-event-label"
  assert_screen_has "Some($event)" "$label-event"
  assert_screen_has "needs_input $capability" "$label-capability"
  assert_screen_has 'Diagnostics retain no prompt, response, command, tool content, or terminal text.' \
    "$label-redaction-note"
  assert_screen_lacks "$E2E_SECRET" "$label-secret"
  send_escape
}

restart_and_rehydrate() {
  local provider="$1"
  stop_client
  start_client
  wait_for_status '▸ running' "$provider-rehydrated" 30
  wait_provider_state "$provider" working provider_integration working true
  state_tool assert-contract "$provider" ||
    fail_agent_e2e "$provider launch contract was not preserved across restart"
  attach_workspace
  capture_screen "$provider-rehydrated-live" >/dev/null
}

exercise_provider() {
  local provider="$1" capability asset
  if [ "$provider" = opencode ]; then
    capability=full
  else
    capability=partial
  fi

  run_tui_command ":agent spawn $provider"
  wait_for_screen "Spawned $provider in background tab" "$provider-spawned" 30
  state_tool wait-meta "$provider" || fail_agent_e2e "$provider did not publish launch metadata"
  state_tool assert-contract "$provider" || fail_agent_e2e "$provider launch contract was invalid"
  asset="$(state_tool field "$provider" asset)"

  wait_provider_state "$provider" ready provider_integration ready true
  wait_for_status '· idle' "$provider-idle" 20

  state_tool control "$provider" working
  wait_provider_state "$provider" working provider_integration working true
  wait_for_status '▸ running' "$provider-running" 20

  state_tool control "$provider" input
  wait_provider_state "$provider" needs_input provider_integration needs_input true
  wait_for_status '! asks' "$provider-asks" 20

  state_tool control "$provider" "done"
  wait_provider_state "$provider" "done" provider_integration turn_completed true
  wait_for_status '✓ done' "$provider-done" 20
  assert_status_evidence "$provider" ProviderIntegration TurnCompleted "$capability" \
    "$provider-done-evidence"
  state_tool assert-redacted "$E2E_SECRET" ||
    fail_agent_e2e "$provider hook payload leaked into host state"

  ensure_work
  send_hex 03
  wait_provider_state "$provider" unknown process_supervisor state_unknown true
  wait_for_screen '? unsure' "$provider-interrupted" 20
  ensure_manage
  wait_for_status '? unsure' "$provider-interrupted-manage" 20
  assert_status_evidence "$provider" ProcessSupervisor StateUnknown "$capability" \
    "$provider-interrupted-evidence"

  state_tool control "$provider" working
  wait_provider_state "$provider" working provider_integration working true
  wait_for_status '▸ running' "$provider-resumed" 20
  restart_and_rehydrate "$provider"

  state_tool control "$provider" exit
  wait_provider_state "$provider" exited process_supervisor exited false
  wait_for_screen '× exited' "$provider-exited" 30
  ensure_manage
  wait_for_status '× exited' "$provider-exited-manage" 20
  wait_for_asset_absent "$asset"
  state_tool assert-redacted "$E2E_SECRET" ||
    fail_agent_e2e "$provider cleanup left sensitive state"
}
