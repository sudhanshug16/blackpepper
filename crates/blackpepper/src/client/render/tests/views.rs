use super::{buffer_text, draw, empty_state, workspace_state};
use crate::client::{ClientCommand, ClientMode};

#[test]
fn first_run_uses_the_four_row_terminal_mark_when_space_permits() {
    let mut state = empty_state();
    let rendered = buffer_text(&draw(&mut state, 80, 24));

    for row in ["█", "█▀▄  █▀▄", "█▄▀  █▄▀", "     █"] {
        assert!(rendered.contains(row));
    }
    // The version lives in the header's right slot, so the panel spends its
    // rows on what you can do here instead of repeating it.
    assert!(!rendered.contains(&format!("blackpepper v{}", env!("CARGO_PKG_VERSION"))));
    assert!(rendered.contains("No workspace is registered here."));
    for (key, description) in [
        (":workspace add", "register a folder"),
        (":host add", "work on a linux ssh host"),
    ] {
        assert!(
            rendered.contains(key) && rendered.contains(description),
            "missing {key} hint in:\n{rendered}"
        );
    }
    for command in [":workspace add ", ":host add "] {
        let targets = state
            .mouse_targets
            .iter()
            .filter(|target| {
                matches!(
                    &target.action,
                    crate::client::state::MouseAction::PrefillCommand(value) if value == command
                )
            })
            .collect::<Vec<_>>();
        assert!(
            targets.iter().any(|target| target.area.x < 32)
                && targets.iter().any(|target| target.area.x >= 32),
            "{command:?} is not clickable in both visible first-run surfaces"
        );
    }
}

#[test]
fn narrow_approval_shows_repository_exact_plan_hooks_and_approve() {
    let mut state = workspace_state();
    let workspace_id = state.selected_workspace.unwrap();
    state.pending_approval = Some(crate::client::state::PendingWorktrunkApproval {
        workspace_id,
        command: ClientCommand::WorktreeCreate {
            branch: "feature/auth".to_owned(),
            base: Some("main".to_owned()),
        },
        approval: crate::worktrunk::WorktrunkApprovalToken {
            schema: 1,
            digest: "0".repeat(64),
        },
        review: "mutation\nwt create feature/auth --base main --foreground --format=json\n\nunapproved project hooks\npost-create: ./scripts/bootstrap.sh\n\napproval binds to this exact Worktrunk command and project hook plan.\n:approve  run · esc dismiss · ↑↓ scroll".to_owned(),
    });

    let rendered = buffer_text(&draw(&mut state, 40, 24));
    for marker in [
        "worktrunk will mutate this",
        "repository",
        "repository",
        "github.com/example/blackpepper",
        "mutation",
        "wt create feature/auth --base main",
        "unapproved project hooks",
        "post-create: ./scripts/bootstrap.sh",
        "approval binds to this exact",
        "Worktrunk command and project hook",
        ":approve",
    ] {
        assert!(
            rendered.contains(marker),
            "missing {marker:?} in:\n{rendered}"
        );
    }
    for old_heading in ["WORKTRUNK MUTATION", "PROJECT COMMANDS", "APPROVAL"] {
        assert!(!rendered.contains(old_heading));
    }
    let approve = state
        .mouse_targets
        .iter()
        .find(|target| matches!(target.action, crate::client::state::MouseAction::Approve))
        .expect("visible approval action");
    let dismiss = state
        .mouse_targets
        .iter()
        .find(|target| {
            matches!(
                target.action,
                crate::client::state::MouseAction::DismissApproval
            )
        })
        .expect("visible approval dismissal");
    assert_eq!(approve.area.y, dismiss.area.y);
    assert_eq!(approve.area.height, 1);
}

#[test]
fn narrow_authentication_names_openssh_ownership_and_storage_boundary() {
    let mut state = workspace_state();
    state.mode = ClientMode::Authenticate;
    state.authentication_output = b"password:".to_vec();

    let terminal = draw(&mut state, 40, 24);
    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("SSH AUTHENTICATION"));
    assert!(rendered.contains("OpenSSH owns authentication."));
    assert!(rendered.contains("Blackpepper stores no credentials."));
    assert!(rendered.contains("nothing is stored"));
    assert!(rendered.contains("password:"));
    assert!(rendered.contains(" AUTHENTICATE "));
}

#[test]
fn narrow_authentication_tails_the_openssh_transcript_to_the_current_prompt() {
    let mut state = workspace_state();
    state.mode = ClientMode::Authenticate;
    state.authentication_output = b"The authenticity of host cannot be established.\nED25519 key fingerprint is SHA256:test.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? yes\nWarning: Permanently added the host.\nPrevious prompt output\nMore previous prompt output\ndev@example.com's password:"
        .to_vec();

    let rendered = buffer_text(&draw(&mut state, 40, 12));
    assert!(rendered.contains("OpenSSH owns authentication."));
    assert!(rendered.contains("Blackpepper stores no credentials."));
    assert!(rendered.contains("dev@example.com's password:"));
    assert!(rendered.contains(" AUTHENTICATE "));
}

#[test]
fn narrow_detail_remains_visible_beside_the_workspace_selector() {
    let mut state = workspace_state();
    state.set_detail("Agent status evidence", "codex ? unsure");

    let rendered = buffer_text(&draw(&mut state, 40, 12));
    assert!(rendered.contains("HOSTS"));
    assert!(rendered.contains("AGENT STATUS EVIDENCE"));
    assert!(rendered.contains("codex ? unsure"));
    assert!(rendered.contains(" MANAGE "));
    assert!(state.mouse_targets.iter().any(|target| matches!(
        target.action,
        crate::client::state::MouseAction::CloseDetail
    )));
}
