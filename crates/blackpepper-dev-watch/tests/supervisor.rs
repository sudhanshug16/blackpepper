#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const FAKE_CARGO: &str = r#"#!/bin/sh
set -eu
source_text=$(cat "$WATCH_TEST_SOURCE")
if printf '%s' "$source_text" | grep -q FAIL; then
  printf '%s\n' 'simulated compile failure'
  exit 1
fi
if printf '%s' "$source_text" | grep -q SLOW; then
  : > "$WATCH_TEST_BUILD_STARTED"
  sleep 30
fi
number=1
if [ -f "$WATCH_TEST_COUNTER" ]; then
  number=$(( $(cat "$WATCH_TEST_COUNTER") + 1 ))
fi
printf '%s' "$number" > "$WATCH_TEST_COUNTER"
mkdir -p "$WATCH_TEST_OUTPUT"
sed "s/@BUILD_ID@/$BLACKPEPPER_BUILD_ID/g" \
  "$WATCH_TEST_CLIENT" > "$WATCH_TEST_OUTPUT/bp"
sed "s/@BUILD_ID@/$BLACKPEPPER_BUILD_ID/g" \
  "$WATCH_TEST_HOST" > "$WATCH_TEST_OUTPUT/bp-host"
chmod 755 "$WATCH_TEST_OUTPUT/bp" "$WATCH_TEST_OUTPUT/bp-host"
"#;

const FAKE_CLIENT: &str = r#"#!/bin/sh
set -eu
if [ "${1:-}" = --version ]; then
  printf '%s\n' 'blackpepper @BUILD_ID@'
  exit 0
fi
events="$WATCH_TEST_EVENTS"
name=$(basename "$(dirname "$0")")
record() { printf '%s:%s\n' "$1" "$name" >> "$events"; }
trap 'record stop; exit 0' HUP INT TERM
record start
while :; do sleep 0.05; done
"#;

const FAKE_HOST: &str = r#"#!/bin/sh
set -eu
if [ "${1:-}" = --version ]; then
  printf '%s\n' 'bp-host @BUILD_ID@'
  exit 0
fi
exit 64
"#;

#[test]
fn help_does_not_start_a_build() {
    let output = Command::new(env!("CARGO_BIN_EXE_bp-dev-watch"))
        .arg("--help")
        .output()
        .expect("watcher help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Nothing is installed and bp-dev is untouched"));
}

#[test]
fn temporary_build_failure_reload_and_shutdown_are_process_correct() {
    let fixture = Fixture::new();
    let mut watcher = fixture.spawn();
    wait_for("initial client", || !fixture.events().is_empty());
    let first = fixture.events()[0].clone();
    assert!(first.starts_with("start:9.9.9-watch."));
    assert!(first.ends_with("-1"));
    assert!(!fixture.root.join("install").exists());
    assert_eq!(fs::read_to_string(&fixture.counter).unwrap(), "1");

    fs::create_dir_all(fixture.root.join("target/ignored")).unwrap();
    fs::write(fixture.root.join("target/ignored/output"), "output").unwrap();
    thread::sleep(Duration::from_millis(250));
    assert_eq!(fs::read_to_string(&fixture.counter).unwrap(), "1");

    fs::write(&fixture.source, "FAIL").unwrap();
    wait_for("failed rebuild", || {
        fs::read_to_string(&fixture.log).is_ok_and(|log| {
            log.contains("simulated compile failure") && log.contains("Error: build exited")
        })
    });
    thread::sleep(Duration::from_millis(150));
    let after_failure = fixture.events();
    assert_eq!(after_failure.as_slice(), std::slice::from_ref(&first));

    fs::write(&fixture.source, "OK 2").unwrap();
    wait_for("successful reload", || fixture.events().len() >= 3);
    let events = fixture.events();
    let first_name = first.strip_prefix("start:").unwrap();
    assert_eq!(events[1], format!("stop:{first_name}"));
    assert!(!fixture
        .target
        .join("blackpepper-dev-watch/bundles")
        .join(first_name)
        .join("bp-watch")
        .exists());
    assert!(events[2].starts_with("start:9.9.9-watch."));
    assert!(events[2].ends_with("-3"));
    assert!(
        fixture
            .target
            .join("blackpepper-dev-watch/bundles")
            .read_dir()
            .unwrap()
            .count()
            >= 2
    );

    fs::write(&fixture.source, "SLOW").unwrap();
    wait_for("slow rebuild", || fixture.build_started.exists());
    let stopped_at = Instant::now();
    watcher.terminate();
    assert!(stopped_at.elapsed() < Duration::from_secs(3));
    for bundle in fs::read_dir(fixture.target.join("blackpepper-dev-watch/bundles")).unwrap() {
        let bundle = bundle.unwrap().path();
        assert!(!bundle.join("bp-watch").exists());
        assert!(bundle.join("bp-host").is_file());
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    target: PathBuf,
    source: PathBuf,
    events_path: PathBuf,
    counter: PathBuf,
    build_started: PathBuf,
    log: PathBuf,
    cargo: PathBuf,
    client: PathBuf,
    host: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_path_buf();
        let source = root.join("crates/blackpepper/src/main.rs");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            root.join("crates/blackpepper/Cargo.toml"),
            "[package]\nname = \"blackpepper\"\nversion = \"9.9.9\"\n",
        )
        .unwrap();
        fs::write(&source, "OK 1").unwrap();
        let cargo = root.join("fake-cargo.sh");
        let client = root.join("fake-client.sh");
        let host = root.join("fake-host.sh");
        write_executable(&cargo, FAKE_CARGO);
        write_executable(&client, FAKE_CLIENT);
        write_executable(&host, FAKE_HOST);
        Self {
            _temporary: temporary,
            target: root.join("target"),
            events_path: root.join("events"),
            counter: root.join("counter"),
            build_started: root.join("build-started"),
            log: root.join("target/watch.log"),
            cargo,
            client,
            host,
            root,
            source,
        }
    }

    fn spawn(&self) -> WatcherChild {
        let output = self.target.join("test-host/debug");
        let child = Command::new(env!("CARGO_BIN_EXE_bp-dev-watch"))
            .current_dir(&self.root)
            .env("BLACKPEPPER_DEV_WATCH_ROOT", &self.root)
            .env("BLACKPEPPER_DEV_WATCH_CARGO", &self.cargo)
            .env("BLACKPEPPER_DEV_WATCH_HOST_TARGET", "test-host")
            // macOS exposes `true` through PATH but does not guarantee the
            // Linux fixture path `/bin/true`.
            .env("BLACKPEPPER_DEV_WATCH_STRIP", "true")
            .env("BLACKPEPPER_DEV_TARGET_DIR", &self.target)
            .env("BLACKPEPPER_DEV_WATCH_LOG", &self.log)
            .env("BLACKPEPPER_DEV_WATCH_DEBOUNCE", "0.06")
            .env("BLACKPEPPER_DEV_WATCH_GRACE", "0.2")
            .env("WATCH_TEST_SOURCE", &self.source)
            .env("WATCH_TEST_COUNTER", &self.counter)
            .env("WATCH_TEST_CLIENT", &self.client)
            .env("WATCH_TEST_HOST", &self.host)
            .env("WATCH_TEST_OUTPUT", output)
            .env("WATCH_TEST_EVENTS", &self.events_path)
            .env("WATCH_TEST_BUILD_STARTED", &self.build_started)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn Rust watcher");
        WatcherChild { child }
    }

    fn events(&self) -> Vec<String> {
        fs::read_to_string(&self.events_path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

struct WatcherChild {
    child: Child,
}

impl WatcherChild {
    fn terminate(&mut self) {
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        wait_for("watcher shutdown", || {
            self.child.try_wait().unwrap().is_some()
        });
    }
}

impl Drop for WatcherChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn wait_for(description: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {description}");
}
