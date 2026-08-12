use uuid::Uuid;

use super::ssh_command;
use super::{ControlSocket, HostCommand, ProcessSpec, SshConfig, TransportError};

const PID_READY_ATTEMPTS: u8 = 20;
const TERM_EXIT_ATTEMPTS: u8 = 10;

/// Build a normal mux session plus a fail-closed session that can stop it.
///
/// OpenSSH does not reliably forward local signals from a non-PTY mux client.
/// The wrapper therefore records the main remote PID and its Linux start time
/// in a private, launch-scoped directory. Cancellation checks both values
/// before signaling, preventing a stale PID file from killing a reused PID.
pub(crate) fn cancellable_session_specs(
    config: &SshConfig,
    socket: &ControlSocket,
    command: &HostCommand,
) -> Result<(ProcessSpec, ProcessSpec), TransportError> {
    let token = Uuid::new_v4().simple().to_string();
    let original = command.remote_shell_line()?;
    let command_line = wrapped_command(&token, &original);
    let cancel_line = cancellation_command(&token);
    Ok((
        ssh_command::session_spec_line(config, socket, command_line, false)?,
        ssh_command::session_spec_line(config, socket, cancel_line, false)?,
    ))
}

pub(super) fn wrapped_command(token: &str, original: &str) -> String {
    let original = shell_words::quote(original);
    format!(
        "umask 077; \
         bp_dir=/tmp/blackpepper-cancel-{token}; \
         if mkdir \"$bp_dir\" 2>/dev/null; then \
           chmod 700 \"$bp_dir\" || exit 126; \
         elif [ -d \"$bp_dir\" ] && chmod 700 \"$bp_dir\" 2>/dev/null; then \
           :; \
         else \
           exit 126; \
         fi; \
         bp_file=\"$bp_dir/pid\"; \
         bp_tmp=\"$bp_dir/pid.tmp\"; \
         bp_cancel=\"$bp_dir/cancel\"; \
         bp_ack=\"$bp_dir/ack\"; \
         bp_cleanup() {{ rm -f \"$bp_file\" \"$bp_tmp\" \"$bp_cancel\" \"$bp_ack\"; rmdir \"$bp_dir\" 2>/dev/null || :; }}; \
         bp_ack_cancel() {{ \
           rm -f \"$bp_file\" \"$bp_tmp\"; \
           : > \"$bp_ack\"; \
           rm -f \"$bp_cancel\"; \
           sleep 2; \
           rm -f \"$bp_ack\"; \
           rmdir \"$bp_dir\" 2>/dev/null || :; \
           exit 143; \
         }}; \
         bp_finish() {{ \
           bp_status=$1; \
           [ -e \"$bp_cancel\" ] && bp_ack_cancel; \
           bp_cleanup; \
           exit \"$bp_status\"; \
         }}; \
         [ -e \"$bp_cancel\" ] && bp_ack_cancel; \
         exec 9<&0 || {{ bp_cleanup; exit 126; }}; \
         sh -c {original} <&9 9<&- & \
         bp_pid=$!; \
         exec 9<&-; \
         trap 'kill -TERM \"$bp_pid\" 2>/dev/null || :; wait \"$bp_pid\"; bp_finish 143' HUP INT TERM; \
         if [ -e \"$bp_cancel\" ]; then \
           kill -TERM \"$bp_pid\" 2>/dev/null || :; \
           wait \"$bp_pid\"; \
           bp_ack_cancel; \
         fi; \
         bp_stat=$(cat \"/proc/$bp_pid/stat\" 2>/dev/null) || \
           {{ if kill -0 \"$bp_pid\" 2>/dev/null; then \
                kill -TERM \"$bp_pid\" 2>/dev/null || :; \
                wait \"$bp_pid\"; \
                bp_finish 126; \
              else \
                wait \"$bp_pid\"; \
                bp_finish $?; \
              fi; }}; \
         bp_rest=$(printf '%s\\n' \"$bp_stat\" | sed 's/^.*) //'); \
         set -- $bp_rest; \
         [ \"$#\" -ge 20 ] || \
           {{ if kill -0 \"$bp_pid\" 2>/dev/null; then \
                kill -TERM \"$bp_pid\" 2>/dev/null || :; \
                wait \"$bp_pid\"; \
                bp_finish 126; \
              else \
                wait \"$bp_pid\"; \
                bp_finish $?; \
              fi; }}; \
         shift 19; \
         bp_start=$1; \
         if ! printf '%s:%s\\n' \"$bp_pid\" \"$bp_start\" > \"$bp_tmp\" || \
            ! mv -f \"$bp_tmp\" \"$bp_file\"; then \
           kill -TERM \"$bp_pid\" 2>/dev/null || :; \
           wait \"$bp_pid\"; \
           bp_finish 126; \
         fi; \
         wait \"$bp_pid\"; \
         bp_status=$?; \
         trap - HUP INT TERM; \
         bp_finish \"$bp_status\""
    )
}

pub(super) fn cancellation_command(token: &str) -> String {
    format!(
        "umask 077; \
         bp_dir=/tmp/blackpepper-cancel-{token}; \
         if mkdir \"$bp_dir\" 2>/dev/null; then \
           chmod 700 \"$bp_dir\" || exit 125; \
         elif [ -d \"$bp_dir\" ] && chmod 700 \"$bp_dir\" 2>/dev/null; then \
           :; \
         else \
           exit 125; \
         fi; \
         bp_file=\"$bp_dir/pid\"; \
         bp_cancel=\"$bp_dir/cancel\"; \
         bp_ack=\"$bp_dir/ack\"; \
         bp_cleanup() {{ rm -f \"$bp_file\" \"$bp_dir/pid.tmp\" \"$bp_cancel\" \"$bp_ack\"; rmdir \"$bp_dir\" 2>/dev/null || :; }}; \
         : > \"$bp_cancel\" || exit 125; \
         bp_i=0; \
         while [ ! -s \"$bp_file\" ] && [ ! -e \"$bp_ack\" ] && [ \"$bp_i\" -lt {PID_READY_ATTEMPTS} ]; do \
           sleep 0.05; \
           bp_i=$((bp_i + 1)); \
         done; \
         [ -e \"$bp_ack\" ] && {{ bp_cleanup; exit 0; }}; \
         [ -s \"$bp_file\" ] || exit 124; \
         IFS=: read -r bp_pid bp_start < \"$bp_file\" || {{ bp_cleanup; exit 125; }}; \
         case \"$bp_pid\" in ''|*[!0-9]*) bp_cleanup; exit 125;; esac; \
         case \"$bp_start\" in ''|*[!0-9]*) bp_cleanup; exit 125;; esac; \
         bp_stat=$(cat \"/proc/$bp_pid/stat\" 2>/dev/null) || {{ bp_cleanup; exit 0; }}; \
         bp_rest=$(printf '%s\\n' \"$bp_stat\" | sed 's/^.*) //'); \
         set -- $bp_rest; \
         [ \"$#\" -ge 20 ] || {{ bp_cleanup; exit 125; }}; \
         shift 19; \
         [ \"$1\" = \"$bp_start\" ] || {{ bp_cleanup; exit 0; }}; \
         kill -TERM \"$bp_pid\" 2>/dev/null || {{ bp_cleanup; exit 0; }}; \
         bp_i=0; \
         while [ -e \"/proc/$bp_pid\" ] && [ \"$bp_i\" -lt {TERM_EXIT_ATTEMPTS} ]; do \
           sleep 0.05; \
           bp_i=$((bp_i + 1)); \
         done; \
         if [ -e \"/proc/$bp_pid\" ]; then \
           bp_stat=$(cat \"/proc/$bp_pid/stat\" 2>/dev/null) || bp_stat=; \
           bp_rest=$(printf '%s\\n' \"$bp_stat\" | sed 's/^.*) //'); \
           set -- $bp_rest; \
           if [ \"$#\" -ge 20 ]; then \
             shift 19; \
             [ \"$1\" = \"$bp_start\" ] && kill -KILL \"$bp_pid\" 2>/dev/null || :; \
           fi; \
         fi; \
         bp_cleanup; \
         exit 0"
    )
}
