#!/bin/sh
# Recover the caller's terminal after running a remote Blackpepper through SSH.
# Native local `bp` with `:host connect` remains the preferred SSH workflow.

set -u

if [ "${BLACKPEPPER_SSH_TERMINAL_GUARD_ACTIVE:-}" = 1 ]; then
  printf '%s\n' 'ssh-terminal-guard: recursive invocation refused' >&2
  exit 126
fi

tty_state=''
ssh_pid=''
signal_status=''
wait_interrupted=0

# `stty -g` is opaque and host-specific. Restoring that exact value preserves
# caller choices that `stty sane` would overwrite.
tty_state=$(stty -g 2>/dev/null < /dev/tty) || tty_state=''

restore_terminal() {
  if [ -n "$tty_state" ]; then
    stty "$tty_state" 2>/dev/null < /dev/tty > /dev/null || :
  fi

  # Mirror only modes Blackpepper may enable. This deliberately avoids RIS
  # (`ESC c`), which would clear the user's primary screen and visible history.
  {
    printf '\033[?1l\033>\033[?2004l'
    printf '\033[?9l\033[?1000l\033[?1002l\033[?1003l'
    printf '\033[?1005l\033[?1006l\033[0m\033[?1049l\033[?25h'
  } 2>/dev/null > /dev/tty || :
}

trap 'restore_terminal' 0

if [ -z "$tty_state" ]; then
  printf '%s\n' \
    'ssh-terminal-guard: /dev/tty is unavailable; run from an interactive terminal' \
    >&2
  restore_terminal
  exit 1
fi

# ShellCheck cannot see that this function is reached through traps.
# shellcheck disable=SC2317
forward_signal() {
  signal=$1
  case $signal in
    HUP) signal_status=129 ;;
    INT) signal_status=130 ;;
    QUIT) signal_status=131 ;;
    TERM) signal_status=143 ;;
  esac
  wait_interrupted=1
  restore_terminal
  if [ -n "$ssh_pid" ]; then
    kill "-$signal" "$ssh_pid" 2>/dev/null || :
  fi
}

trap 'forward_signal HUP' HUP
trap 'forward_signal INT' INT
trap 'forward_signal QUIT' QUIT
trap 'forward_signal TERM' TERM

ssh_program=$(command -v ssh 2>/dev/null) || ssh_program=''

if [ -n "$signal_status" ]; then
  trap - HUP INT QUIT TERM
  exit "$signal_status"
fi

if [ -z "$ssh_program" ]; then
  ssh_status=127
  printf '%s\n' 'ssh-terminal-guard: system ssh was not found on PATH' >&2
  trap - HUP INT QUIT TERM
  exit "$ssh_status"
fi

# An explicit stdin duplication prevents a POSIX asynchronous command from
# receiving /dev/null. No new process group is created, and signals target only
# this exact child PID—never the caller's shell or an unrelated SSH process.
BLACKPEPPER_SSH_TERMINAL_GUARD_ACTIVE=1 \
  "$ssh_program" "$@" <&0 &
ssh_pid=$!

# A signal caught between launch and `$!` assignment could not be forwarded by
# the trap. Forward it now that the exact owned child PID is available.
if [ -n "$signal_status" ]; then
  case $signal_status in
    129) kill -HUP "$ssh_pid" 2>/dev/null || : ;;
    130) kill -INT "$ssh_pid" 2>/dev/null || : ;;
    131) kill -QUIT "$ssh_pid" 2>/dev/null || : ;;
    143) kill -TERM "$ssh_pid" 2>/dev/null || : ;;
  esac
fi

# A caught signal may interrupt `wait` before ssh has exited. The trap marks
# that interruption; no liveness probe is used after `wait`, so a reaped PID
# can never be mistaken for a newly reused process.
while :; do
  wait_interrupted=0
  wait "$ssh_pid"
  ssh_status=$?
  if [ "$wait_interrupted" -eq 0 ]; then
    ssh_pid=''
    break
  fi
done

if [ -n "$signal_status" ]; then
  ssh_status=$signal_status
fi

trap - HUP INT QUIT TERM
exit "$ssh_status"
