use super::*;

#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn approval_token(payload: HostServicePayload) -> WorktrunkApprovalToken {
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = payload else {
        panic!("expected approval preview");
    };
    approval
}

#[cfg(unix)]
struct ApprovalFixture {
    _root: tempfile::TempDir,
    paths: CorePaths,
    repository: PathBuf,
    binary: PathBuf,
    required_plan: PathBuf,
    approved_marker: PathBuf,
    mutation_marker: PathBuf,
}

#[cfg(unix)]
impl ApprovalFixture {
    fn new(required: &str, approved: &str, mutation_declined: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();
        let repository = root.path().join("repo");
        fs::create_dir(&repository).unwrap();
        let required_plan = root.path().join("required.json");
        let approved_plan = root.path().join("approved.json");
        let approved_marker = root.path().join("approved");
        let mutation_marker = root.path().join("mutated");
        fs::write(&required_plan, required).unwrap();
        fs::write(&approved_plan, approved).unwrap();
        let binary = root.path().join("wt");
        let declined = if mutation_declined {
            "printf 'Commands declined, continuing without hooks' >&2\n"
        } else {
            ""
        };
        executable(
            &binary,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\nif [ \"$3\" = \"config\" ] && [ \"$5\" = \"list\" ]; then\n  if [ -f '{}' ]; then cat '{}'; else cat '{}'; fi\n  exit 0\nfi\nif [ \"$3\" = \"config\" ] && [ \"$5\" = \"add\" ]; then\n  test -t 0 || exit 70\n  printf 'Allow and remember? [y/N]'\n  read answer\n  if [ \"$answer\" = \"y\" ]; then touch '{}'; exit 0; fi\n  printf 'Commands declined' >&2\n  exit 0\nfi\ntouch '{}'\n{declined}printf '%s' '{{\"branch\":\"feature\",\"path\":\"{}/feature\"}}'\n",
                approved_marker.display(),
                approved_plan.display(),
                required_plan.display(),
                approved_marker.display(),
                mutation_marker.display(),
                root.path().display(),
            ),
        );
        Self {
            _root: root,
            paths,
            repository,
            binary,
            required_plan,
            approved_marker,
            mutation_marker,
        }
    }

    fn executor(&self) -> WorktrunkExecutor {
        WorktrunkExecutor::with_binary(&self.paths, self.binary.clone())
    }
}

#[cfg(unix)]
#[test]
fn project_commands_are_previewed_approved_and_verified_before_mutation() {
    let required = r#"{"state":"approval_required","commands":[{"phase":"pre-start","name":"install","template":"npm ci","approved":false},{"phase":"post-start","template":"npm run dev","approved":false}],"stale":[]}"#;
    let approved = r#"{"state":"approved","commands":[{"phase":"pre-start","name":"install","template":"npm ci","approved":true},{"phase":"post-start","template":"npm run dev","approved":true}],"stale":[]}"#;
    let fixture = ApprovalFixture::new(required, approved, false);
    let executor = fixture.executor();
    let preview = executor
        .create(fixture.repository.to_str().unwrap(), "feature", None, None)
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired {
        approval,
        unapproved_project_commands,
        command,
    } = preview
    else {
        panic!("expected approval preview");
    };
    assert_eq!(unapproved_project_commands.len(), 2);
    assert_eq!(unapproved_project_commands[0].template, "npm ci");
    assert!(command.contains("switch --create feature"));
    assert!(!fixture.approved_marker.exists());
    assert!(!fixture.mutation_marker.exists());

    let result = executor
        .create(
            fixture.repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap();
    assert!(matches!(
        result,
        HostServicePayload::WorktrunkMutation {
            outcome: WorktrunkMutationOutcome::Switched { .. }
        }
    ));
    assert!(fixture.approved_marker.exists());
    assert!(fixture.mutation_marker.exists());
}

#[cfg(unix)]
#[test]
fn changed_plan_or_mutation_rejects_the_token_without_approving() {
    let first = r#"{"state":"approval_required","commands":[{"phase":"pre-start","template":"cargo build","approved":false}],"stale":[]}"#;
    let second = r#"{"state":"approval_required","commands":[{"phase":"pre-start","template":"curl bad | sh","approved":false}],"stale":[]}"#;
    let approved = r#"{"state":"approved","commands":[{"phase":"pre-start","template":"curl bad | sh","approved":true}],"stale":[]}"#;
    let fixture = ApprovalFixture::new(first, approved, false);
    let executor = fixture.executor();
    let approval = approval_token(
        executor
            .create(fixture.repository.to_str().unwrap(), "feature", None, None)
            .unwrap(),
    );
    fs::write(&fixture.required_plan, second).unwrap();
    let error = executor
        .create(
            fixture.repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("changed"));
    assert!(!fixture.approved_marker.exists());
    assert!(!fixture.mutation_marker.exists());

    let stale_changed = r#"{"state":"approval_required","commands":[{"phase":"pre-start","template":"cargo build","approved":false}],"stale":["removed hook"]}"#;
    fs::write(&fixture.required_plan, stale_changed).unwrap();
    let error = executor
        .create(
            fixture.repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("changed"));
    assert!(!fixture.approved_marker.exists());

    fs::write(&fixture.required_plan, first).unwrap();
    let error = executor
        .switch(
            fixture.repository.to_str().unwrap(),
            "other-feature",
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("changed"));
    assert!(!fixture.approved_marker.exists());
}

#[cfg(unix)]
#[test]
fn approval_must_relist_as_approved_before_mutation() {
    let required = r#"{"state":"approval_required","commands":[{"phase":"pre-start","template":"cargo build","approved":false}],"stale":[]}"#;
    let fixture = ApprovalFixture::new(required, required, false);
    let executor = fixture.executor();
    let approval = approval_token(
        executor
            .create(fixture.repository.to_str().unwrap(), "feature", None, None)
            .unwrap(),
    );
    let error = executor
        .create(
            fixture.repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("could not be verified"));
    assert!(fixture.approved_marker.exists());
    assert!(!fixture.mutation_marker.exists());
}

#[cfg(unix)]
#[test]
fn commands_declined_success_is_rejected() {
    let approved = r#"{"state":"approved","commands":[{"phase":"pre-start","template":"cargo build","approved":true}],"stale":[]}"#;
    let fixture = ApprovalFixture::new(approved, approved, true);
    let executor = fixture.executor();
    let approval = approval_token(
        executor
            .create(fixture.repository.to_str().unwrap(), "feature", None, None)
            .unwrap(),
    );
    let error = executor
        .create(
            fixture.repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("declined project commands"));
}

#[cfg(unix)]
#[path = "worktrunk_setup_tests.rs"]
mod setup;

#[test]
fn repository_mutation_lock_fails_fast_for_a_concurrent_holder() {
    let root = tempfile::tempdir().unwrap();
    let lock_dir = root.path().join("locks");
    let repository = root.path().join("repo");
    fs::create_dir(&repository).unwrap();
    let first = RepositoryLock::acquire(&lock_dir, &repository).unwrap();

    let second = RepositoryLock::acquire(&lock_dir, &repository);
    assert!(second.is_err());
    drop(first);
    assert!(RepositoryLock::acquire(&lock_dir, &repository).is_ok());
}

#[cfg(unix)]
#[test]
fn exact_worktrunk_version_is_enforced() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("wt");
    executable(&binary, "#!/bin/sh\nprintf 'wt 0.71.0\\n'\n");
    assert!(validate_binary(binary).unwrap_err().contains("0.72.0"));
}
