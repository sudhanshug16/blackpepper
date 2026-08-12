use super::*;

#[test]
fn failed_setup_keeps_and_reports_the_created_folder() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let repository = root.path().join("repo");
    let created = root.path().join("feature");
    fs::create_dir(&repository).unwrap();
    fs::create_dir(&created).unwrap();
    let binary = root.path().join("wt");
    executable(
        &binary,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\nif [ \"$3\" = \"config\" ]; then printf '%s' '{{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}}'; exit 0; fi\nprintf '%s' '{{\"branch\":\"feature\",\"path\":\"{}\"}}'\nprintf 'setup hook failed' >&2\nexit 1\n",
            created.display()
        ),
    );
    let executor = WorktrunkExecutor::with_binary(&paths, binary);

    let preview = executor
        .create(repository.to_str().unwrap(), "feature", None, None)
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };
    let result = executor
        .create(
            repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap();
    assert!(matches!(
        result,
        HostServicePayload::WorktrunkMutation {
            outcome: WorktrunkMutationOutcome::SetupFailed { path, message }
        } if path == created && message.contains("setup hook failed")
    ));
    assert!(created.is_dir());
}

#[test]
fn failed_setup_without_switch_json_recovers_one_new_listed_folder() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let repository = root.path().join("repo");
    let created = root.path().join("feature");
    fs::create_dir(&repository).unwrap();
    let binary = root.path().join("wt");
    executable(
        &binary,
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\n\
             if [ \"$3\" = \"config\" ]; then printf '%s' '{{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}}'; exit 0; fi\n\
             if [ \"$5\" = \"list\" ]; then\n\
               if [ -d '{}' ]; then\n\
                 printf '%s' '{{\"schema\":2,\"repo\":{{\"default_branch\":\"main\"}},\"items\":[{{\"branch\":\"main\",\"worktree\":{{\"path\":\"{}\"}}}},{{\"branch\":\"feature\",\"worktree\":{{\"path\":\"{}\"}}}}]}}'\n\
               else\n\
                 printf '%s' '{{\"schema\":2,\"repo\":{{\"default_branch\":\"main\"}},\"items\":[{{\"branch\":\"main\",\"worktree\":{{\"path\":\"{}\"}}}}]}}'\n\
               fi\n\
               exit 0\n\
             fi\n\
             mkdir '{}'\n\
             printf 'setup hook failed without JSON' >&2\n\
             exit 1\n",
            created.display(),
            repository.display(),
            created.display(),
            repository.display(),
            created.display(),
        ),
    );
    let executor = WorktrunkExecutor::with_binary(&paths, binary);
    let preview = executor
        .create(repository.to_str().unwrap(), "feature", None, None)
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };

    let result = executor
        .create(
            repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap();

    assert!(matches!(
        result,
        HostServicePayload::WorktrunkMutation {
            outcome: WorktrunkMutationOutcome::SetupFailed { path, message }
        } if path == created && message.contains("without JSON")
    ));
}

#[test]
fn failed_setup_without_json_refuses_ambiguous_new_folders() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let repository = root.path().join("repo");
    let first = root.path().join("first");
    let second = root.path().join("second");
    fs::create_dir(&repository).unwrap();
    let binary = root.path().join("wt");
    executable(
        &binary,
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\n\
             if [ \"$3\" = \"config\" ]; then printf '%s' '{{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}}'; exit 0; fi\n\
             if [ \"$5\" = \"list\" ]; then\n\
               if [ -d '{}' ]; then\n\
                 printf '%s' '{{\"schema\":2,\"repo\":{{\"default_branch\":\"main\"}},\"items\":[{{\"worktree\":{{\"path\":\"{}\"}}}},{{\"worktree\":{{\"path\":\"{}\"}}}},{{\"worktree\":{{\"path\":\"{}\"}}}}]}}'\n\
               else\n\
                 printf '%s' '{{\"schema\":2,\"repo\":{{\"default_branch\":\"main\"}},\"items\":[{{\"worktree\":{{\"path\":\"{}\"}}}}]}}'\n\
               fi\n\
               exit 0\n\
             fi\n\
             mkdir '{}' '{}'\n\
             printf 'setup failed ambiguously' >&2\n\
             exit 1\n",
            first.display(),
            repository.display(),
            first.display(),
            second.display(),
            repository.display(),
            first.display(),
            second.display(),
        ),
    );
    let executor = WorktrunkExecutor::with_binary(&paths, binary);
    let preview = executor
        .create(repository.to_str().unwrap(), "feature", None, None)
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };

    let error = executor
        .create(
            repository.to_str().unwrap(),
            "feature",
            None,
            Some(&approval),
        )
        .unwrap_err();

    assert!(error.contains("setup failed ambiguously"));
    assert!(first.is_dir() && second.is_dir());
}
