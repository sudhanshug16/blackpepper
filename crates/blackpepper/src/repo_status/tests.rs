use super::{
    parse_branch_head, parse_dirty, parse_divergence, parse_pr_view, should_fetch_pr_status,
    Divergence, PrState,
};
use std::time::{Duration, Instant};

#[test]
fn parse_divergence_extracts_counts() {
    let output = "# branch.oid 0123\n# branch.ab +2 -1\n";
    let result = parse_divergence(output).expect("divergence");
    assert_eq!(result.ahead, 2);
    assert_eq!(result.behind, 1);
}

#[test]
fn parse_divergence_ignores_zero() {
    let output = "# branch.ab +0 -0\n";
    assert!(parse_divergence(output).is_none());
}

#[test]
fn parse_branch_head_prefers_name() {
    let output = "# branch.head main\n";
    assert_eq!(parse_branch_head(output), Some("main".to_string()));
}

#[test]
fn parse_branch_head_handles_detached() {
    let output = "# branch.head (detached)\n";
    assert_eq!(parse_branch_head(output), Some("detached".to_string()));
}

#[test]
fn parse_dirty_detects_changes() {
    let output = "# branch.head main\n1 .M N... 100644 100644 100644 abc abc file.txt\n";
    assert!(parse_dirty(output));
}

#[test]
fn parse_dirty_ignores_clean_status() {
    let output = "# branch.head main\n# branch.ab +0 -0\n";
    assert!(!parse_dirty(output));
}

#[test]
fn parse_pr_view_merges_state() {
    let raw =
        r#"{"number":12,"title":"Ship it","state":"CLOSED","mergedAt":"2024-01-01T00:00:00Z"}"#;
    let info = parse_pr_view(raw).expect("parse ok");
    assert_eq!(info.number, 12);
    assert_eq!(info.title, "Ship it");
    assert!(matches!(info.state, PrState::Merged));
}

#[test]
fn parse_pr_view_closed_state() {
    let raw = r#"{"number":12,"title":"Nope","state":"CLOSED","mergedAt":null}"#;
    let info = parse_pr_view(raw).expect("parse ok");
    assert!(matches!(info.state, PrState::Closed));
}

#[test]
fn parse_pr_view_open_state() {
    let raw = r#"{"number":12,"title":"Yep","state":"OPEN","mergedAt":null}"#;
    let info = parse_pr_view(raw).expect("parse ok");
    assert!(matches!(info.state, PrState::Open));
}

#[test]
fn parse_pr_view_draft_state() {
    let raw = r#"{"number":12,"title":"Draft","state":"OPEN","mergedAt":null,"isDraft":true}"#;
    let info = parse_pr_view(raw).expect("parse ok");
    assert!(matches!(info.state, PrState::Draft));
}

#[test]
fn divergence_struct_stays_simple() {
    let divergence = Divergence {
        ahead: 1,
        behind: 0,
    };
    assert_eq!(divergence.ahead, 1);
    assert_eq!(divergence.behind, 0);
}

#[test]
fn should_fetch_pr_status_first_time() {
    assert!(should_fetch_pr_status(None, Instant::now(), false));
}

#[test]
fn should_fetch_pr_status_respects_rate_limit() {
    let now = Instant::now();
    let recent = now - Duration::from_secs(60);
    assert!(!should_fetch_pr_status(Some(recent), now, false));
}

#[test]
fn should_fetch_pr_status_after_interval() {
    let now = Instant::now();
    let old = now - Duration::from_secs(301);
    assert!(should_fetch_pr_status(Some(old), now, false));
}

#[test]
fn should_fetch_pr_status_when_forced() {
    let now = Instant::now();
    let recent = now - Duration::from_secs(60);
    assert!(should_fetch_pr_status(Some(recent), now, true));
}
