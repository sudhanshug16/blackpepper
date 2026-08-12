use super::{model::WorktreeList, MutationResult, OperationKind, Worktrunk};
use std::path::{Path, PathBuf};

#[test]
fn list_forces_schema_two_and_includes_branches_and_remotes() {
    let spec = Worktrunk::new("wt").list(Path::new("/repo"));
    let args: Vec<_> = spec
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(spec.kind, OperationKind::Read);
    assert_eq!(
        args,
        [
            "-C",
            "/repo",
            "--config-set",
            "list.json-schema=2",
            "list",
            "--branches",
            "--remotes",
            "--format=json"
        ]
    );
}

#[test]
fn create_preserves_hooks_and_never_auto_approves() {
    let spec = Worktrunk::new("wt")
        .create(Path::new("/repo"), "feature/auth", Some("main"))
        .unwrap();
    let rendered = spec.display();
    assert_eq!(spec.kind, OperationKind::Mutation);
    assert!(spec.interactive);
    assert!(rendered.contains("switch --create feature/auth --base main"));
    assert!(!rendered.contains("--yes"));
    assert!(!rendered.contains("--no-hooks"));
    assert!(!spec.contains_forbidden_force_flag());
}

#[test]
fn switch_accepts_pr_and_mr_selectors() {
    for selector in ["pr:123", "mr:42", "https://github.com/o/r/pull/7"] {
        let spec = Worktrunk::new("wt")
            .switch(Path::new("/repo"), selector)
            .unwrap();
        assert!(spec.display().contains(selector));
    }
}

#[test]
fn selectors_cannot_be_smuggled_as_options() {
    let client = Worktrunk::new("wt");
    for selector in ["--force", "--clobber", "-f"] {
        assert!(client.switch(Path::new("/repo"), selector).is_err());
        assert!(client.create(Path::new("/repo"), selector, None).is_err());
    }
}

#[test]
fn remove_is_foreground_absolute_and_non_forcing() {
    let client = Worktrunk::new("wt");
    assert!(client
        .remove(Path::new("/repo"), Path::new("relative"))
        .is_err());
    let spec = client
        .remove(Path::new("/repo"), Path::new("/repo.feature"))
        .unwrap();
    assert!(spec.display().contains("--foreground --format=json"));
    assert!(!spec.contains_forbidden_force_flag());
}

#[test]
fn approval_commands_are_exact_and_never_bypass_prompts() {
    let client = Worktrunk::new("wt");
    let list = client.approval_list(Path::new("/repo"));
    assert_eq!(
        list.args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        [
            "-C",
            "/repo",
            "config",
            "approvals",
            "list",
            "--format=json"
        ]
    );
    let add = client.approval_add(Path::new("/repo"));
    assert_eq!(
        add.args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["-C", "/repo", "config", "approvals", "add"]
    );
    assert!(add.interactive);
    for forbidden in ["--yes", "-y", "--no-hooks", "--force"] {
        assert!(!add.display().contains(forbidden));
    }
}

#[test]
fn schema_two_is_required() {
    let invalid = r#"{"schema":1,"repo":{"default_branch":"main"},"items":[]}"#;
    assert!(WorktreeList::parse(invalid)
        .unwrap_err()
        .contains("expected 2"));
}

#[test]
fn schema_two_fixture_parses_worktree_and_branch_rows() {
    let fixture = r#"{
      "schema": 2,
      "repo": {"default_branch":"main","forge":{"url":"https://github.com/o/r","provider":"github","host":"github.com","owner":"o","name":"r","remote":"origin"}},
      "collected": {"ci":false,"summary":false},
      "items": [
        {"branch":"feature","head":{"sha":"abc","short_sha":"abc","subject":"work","committed_at":"2026-01-01T00:00:00Z"},"worktree":{"path":"/repo.feature","main":false,"current":true,"previous":false,"detached":false,"branch_mismatch":false,"duplicate_branch":false,"changes":{"modified":true}},"display":{"state":"ahead"}},
        {"branch":"remote-only","remote":"origin","head":null}
      ]
    }"#;
    let parsed = WorktreeList::parse(fixture).unwrap();
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(
        parsed.items[0].worktree.as_ref().unwrap().path,
        PathBuf::from("/repo.feature")
    );
    assert!(
        parsed.items[0]
            .worktree
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .modified
    );
    assert!(parsed.items[1].worktree.is_none());
}

#[test]
fn approval_plan_is_strict_bounded_and_state_consistent() {
    let valid = br#"{"state":"approval_required","commands":[{"phase":"post-start","name":"dev","template":"npm run dev","approved":false}],"stale":["old command"]}"#;
    let parsed = super::model::WorktrunkApprovalPlan::parse(valid).unwrap();
    assert_eq!(parsed.unapproved_commands().len(), 1);

    let inconsistent = br#"{"state":"approved","commands":[{"phase":"post-start","template":"npm run dev","approved":false}],"stale":[]}"#;
    assert!(super::model::WorktrunkApprovalPlan::parse(inconsistent).is_err());
    let unknown = br#"{"state":"no_commands","commands":[],"stale":[],"payload":"secret"}"#;
    assert!(super::model::WorktrunkApprovalPlan::parse(unknown).is_err());
    let oversized = format!(
        "{{\"state\":\"approval_required\",\"commands\":[{{\"phase\":\"post-start\",\"template\":{},\"approved\":false}}],\"stale\":[]}}",
        serde_json::to_string(&"x".repeat(64 * 1024 + 1)).unwrap()
    );
    assert!(super::model::WorktrunkApprovalPlan::parse(oversized.as_bytes()).is_err());
    assert!(super::model::WorktrunkApprovalPlan::parse(&vec![b' '; 1024 * 1024 + 1]).is_err());
    let too_many = serde_json::json!({
        "state": "approval_required",
        "commands": vec![serde_json::json!({
            "phase": "post-start",
            "template": "true",
            "approved": false
        }); 1025],
        "stale": []
    });
    assert!(super::model::WorktrunkApprovalPlan::parse(
        serde_json::to_vec(&too_many).unwrap().as_slice()
    )
    .is_err());
}

#[test]
fn disconnect_outcome_is_explicit_and_not_success() {
    let outcome: MutationResult<()> = MutationResult::UnknownAfterDisconnect;
    assert_ne!(outcome, MutationResult::Success(()));
}
