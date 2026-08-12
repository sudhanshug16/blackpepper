use super::approval_review;
use crate::worktrunk::WorktrunkProjectCommand;

#[test]
fn approval_review_uses_v2_hierarchy_without_changing_exact_plan_content() {
    let command = "wt create feature/auth --base main --foreground --format=json";
    let hook = "./scripts/bootstrap.sh --locked";
    let review = approval_review(
        command,
        &[WorktrunkProjectCommand {
            phase: "post-create".to_owned(),
            name: Some("bootstrap".to_owned()),
            template: hook.to_owned(),
        }],
    );

    for required in [
        "mutation",
        command,
        "unapproved project hooks",
        "post-create / bootstrap: ./scripts/bootstrap.sh --locked",
        "approval binds to this exact Worktrunk command and project hook plan.",
        ":approve  run · esc dismiss · ↑↓ scroll",
    ] {
        assert!(review.contains(required), "missing {required:?}");
    }
    assert!(review.contains(hook));
    assert!(!review.contains("WORKTRUNK MUTATION"));
    assert!(!review.contains("PROJECT COMMAND"));
}

#[test]
fn approval_review_truthfully_handles_an_empty_hook_plan() {
    let review = approval_review("wt remove --foreground --format=json", &[]);
    assert!(review.contains("unapproved project hooks\nnone"));
    assert!(review.contains(":approve"));
}
