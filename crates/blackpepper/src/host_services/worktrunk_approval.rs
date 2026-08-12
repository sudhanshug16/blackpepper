use super::process::GuardedPtyProcess;
use super::worktrunk_exec::{execute, require_success};
use super::worktrunk_lock::{repository_identity, RepositoryLock};
use crate::core::HostServicePayload;
use crate::transport::ProcessSpec;
use crate::worktrunk::{CommandSpec, Worktrunk, WorktrunkApprovalPlan, WorktrunkApprovalToken};
use portable_pty::PtySize;
use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

const APPROVAL_PROMPT: &[u8] = b"Allow and remember?";
const MAX_APPROVAL_PTY_BYTES: usize = 2 * 1024 * 1024;

pub(super) enum ApprovalDecision {
    Required(Box<HostServicePayload>),
    Authorized(RepositoryLock),
}

/// Verifies the operation and project-command plan under the same lock that
/// remains held through the eventual Worktrunk mutation.
pub(super) fn authorize(
    binary: &Path,
    lock_dir: &Path,
    repository: &Path,
    mutation: &CommandSpec,
    supplied: Option<&WorktrunkApprovalToken>,
) -> Result<ApprovalDecision, String> {
    let lock = RepositoryLock::acquire(lock_dir, repository)?;
    let plan = load_plan(&lock, binary, repository)?;
    let expected = approval_token(repository, mutation, &plan)?;
    let Some(supplied) = supplied else {
        return Ok(ApprovalDecision::Required(Box::new(
            HostServicePayload::WorktrunkApprovalRequired {
                command: mutation.display(),
                approval: expected,
                unapproved_project_commands: plan.unapproved_commands(),
            },
        )));
    };
    if !supplied.is_well_formed() || supplied != &expected {
        return Err(changed_plan_message());
    }

    if plan.requires_approval() {
        approve_exact_plan(&lock, binary, repository, mutation, &plan, &expected)?;
    }
    Ok(ApprovalDecision::Authorized(lock))
}

fn load_plan(
    lock: &RepositoryLock,
    binary: &Path,
    repository: &Path,
) -> Result<WorktrunkApprovalPlan, String> {
    let spec = Worktrunk::new(binary).approval_list(repository);
    let output = execute(lock, &spec)?;
    require_success(&spec, &output)?;
    if output.truncated {
        return Err("Worktrunk approval JSON exceeded the safe capture limit.".to_owned());
    }
    WorktrunkApprovalPlan::parse(&output.stdout)
}

fn approve_exact_plan(
    lock: &RepositoryLock,
    binary: &Path,
    repository: &Path,
    mutation: &CommandSpec,
    reviewed: &WorktrunkApprovalPlan,
    reviewed_token: &WorktrunkApprovalToken,
) -> Result<(), String> {
    let spec = Worktrunk::new(binary).approval_add(repository);
    if spec.contains_forbidden_force_flag()
        || spec
            .args
            .iter()
            .any(|argument| matches!(argument.to_str(), Some("--yes" | "-y" | "--no-hooks")))
    {
        return Err("Unsafe Worktrunk approval option was blocked.".to_owned());
    }
    let process = ProcessSpec::new(&spec.program).args(spec.args.clone());
    let mut pty = GuardedPtyProcess::spawn(
        lock,
        &process,
        PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        },
    )
    .map_err(|error| format!("Could not start Worktrunk approval: {error}"))?;
    let mut reader = pty
        .take_reader()
        .map_err(|error| format!("Could not read Worktrunk approval: {error}"))?;
    let prompt = read_until_prompt(&mut reader)
        .map_err(|error| format!("Could not read Worktrunk approval: {error}"))?;

    if prompt == PromptRead::Prompt {
        // Worktrunk has now loaded the exact command batch it will save. A
        // second read-only list closes the config-read race before answering.
        let current = load_plan(lock, binary, repository)?;
        let current_token = approval_token(repository, mutation, &current)?;
        if &current_token != reviewed_token {
            let _ = pty.write_all(b"n\n");
            let _ = drain_bounded(&mut reader);
            let _ = pty.wait();
            return Err(changed_plan_message());
        }
        pty.write_all(b"y\n")
            .map_err(|error| format!("Could not confirm Worktrunk approval: {error}"))?;
        drain_bounded(&mut reader)
            .map_err(|error| format!("Could not finish Worktrunk approval: {error}"))?;
    }

    let exit = pty
        .wait()
        .map_err(|error| format!("Could not wait for Worktrunk approval: {error}"))?;
    if !exit.success {
        return Err("Worktrunk did not persist project-command approval.".to_owned());
    }
    let verified = load_plan(lock, binary, repository)?;
    if !verified.is_approved() || !reviewed.same_commands_and_stale(&verified) {
        return Err(
            "Worktrunk project-command approval could not be verified; no mutation was run."
                .to_owned(),
        );
    }
    Ok(())
}

fn approval_token(
    repository: &Path,
    mutation: &CommandSpec,
    plan: &WorktrunkApprovalPlan,
) -> Result<WorktrunkApprovalToken, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"blackpepper-worktrunk-approval-v1\0");
    hash_field(
        &mut hasher,
        repository_identity(repository)
            .as_os_str()
            .as_encoded_bytes(),
    );
    hash_field(&mut hasher, repository.as_os_str().as_encoded_bytes());
    for argument in &mutation.args {
        hash_field(&mut hasher, argument.as_os_str().as_encoded_bytes());
    }
    let plan = serde_json::to_vec(plan)
        .map_err(|error| format!("Could not encode Worktrunk approval plan: {error}"))?;
    hash_field(&mut hasher, &plan);
    Ok(WorktrunkApprovalToken::new(format!(
        "{:x}",
        hasher.finalize()
    )))
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromptRead {
    Prompt,
    Eof,
}

fn read_until_prompt(reader: &mut impl Read) -> io::Result<PromptRead> {
    let mut tail = Vec::new();
    let mut total = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.raw_os_error() == Some(libc::EIO) => 0,
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(PromptRead::Eof);
        }
        total = total.saturating_add(read);
        if total > MAX_APPROVAL_PTY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Worktrunk approval output exceeded 2 MiB",
            ));
        }
        tail.extend_from_slice(&buffer[..read]);
        if tail
            .windows(APPROVAL_PROMPT.len())
            .any(|window| window == APPROVAL_PROMPT)
        {
            return Ok(PromptRead::Prompt);
        }
        let retained = APPROVAL_PROMPT.len().saturating_sub(1);
        if tail.len() > retained {
            tail.drain(..tail.len() - retained);
        }
    }
}

fn drain_bounded(reader: &mut impl Read) -> io::Result<()> {
    let mut total = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.raw_os_error() == Some(libc::EIO) => 0,
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(());
        }
        total = total.saturating_add(read);
        if total > MAX_APPROVAL_PTY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Worktrunk approval output exceeded 2 MiB",
            ));
        }
    }
}

fn changed_plan_message() -> String {
    "Worktrunk project commands or the requested mutation changed; review approval again."
        .to_owned()
}
