use super::*;
use crate::core::HostTransport;
use fs2::FileExt;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

#[test]
fn background_registry_open_can_cancel_while_initialization_is_locked() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    drop(HostRegistry::open(&path).unwrap());
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".init.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::path::PathBuf::from(lock_path))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let entered = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_path = path.clone();
    let worker = std::thread::spawn(move || {
        worker_entered.wait();
        HostRegistry::open_interruptible(worker_path, || worker_cancelled.load(Ordering::SeqCst))
    });
    entered.wait();

    cancelled.store(true, Ordering::SeqCst);

    assert!(matches!(
        worker.join().unwrap(),
        Err(RegistryError::Interrupted(_))
    ));
}

#[test]
fn transient_existing_connections_share_wal_with_the_interactive_registry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut interactive = HostRegistry::open(&path).unwrap();
    let local_id = interactive.ensure_local_host("local").unwrap();

    for index in 0..4 {
        let worker = HostRegistry::open_existing_interruptible(&path, || false).unwrap();
        let remote = HostRecord::new(
            format!("remote-{index}"),
            HostTransport::Ssh {
                destination: format!("remote-{index}.invalid"),
            },
        );
        worker.upsert_host(&remote).unwrap();
        assert!(worker.snapshot().unwrap().hosts.contains(&remote));
        drop(worker);
        assert!(interactive.snapshot().unwrap().hosts.contains(&remote));
    }

    assert_eq!(interactive.local_host_id().unwrap(), local_id);
}

#[test]
fn transient_wal_connections_can_be_dropped_by_the_interactive_thread() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut interactive = HostRegistry::open(&path).unwrap();
    interactive.ensure_local_host("local").unwrap();

    for index in 0..4 {
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            let registry =
                HostRegistry::open_existing_interruptible(&worker_path, || false).unwrap();
            let remote = HostRecord::new(
                format!("remote-{index}"),
                HostTransport::Ssh {
                    destination: format!("remote-{index}.invalid"),
                },
            );
            registry.upsert_host(&remote).unwrap();
            registry.snapshot().unwrap();
            registry
        })
        .join()
        .unwrap();
        // Restore outcomes are sent to the runner and their transient registry
        // connection is dropped there after the SSH transport is merged.
        drop(worker);
        assert_eq!(interactive.journal_mode().unwrap(), "wal");
        assert_eq!(interactive.snapshot().unwrap().hosts.len(), index + 2);
    }
}

#[test]
fn concurrent_transient_wal_readers_do_not_disrupt_import_writes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut interactive = HostRegistry::open(&path).unwrap();
    interactive.ensure_local_host("local").unwrap();
    let start = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for worker_index in 0..2 {
        let worker_path = path.clone();
        let worker_start = Arc::clone(&start);
        workers.push(std::thread::spawn(move || {
            let registry =
                HostRegistry::open_existing_interruptible(worker_path, || false).unwrap();
            worker_start.wait();
            for index in 0..20 {
                let remote = HostRecord::new(
                    format!("remote-{worker_index}-{index}"),
                    HostTransport::Ssh {
                        destination: format!("remote-{worker_index}-{index}.invalid"),
                    },
                );
                registry.upsert_host(&remote).unwrap();
                registry.snapshot().unwrap();
            }
            registry
        }));
    }
    start.wait();
    for _ in 0..40 {
        interactive.snapshot().unwrap();
    }
    for worker in workers {
        drop(worker.join().unwrap());
    }
    assert_eq!(interactive.snapshot().unwrap().hosts.len(), 41);
}

#[test]
fn transient_wal_connections_survive_child_process_spawns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut interactive = HostRegistry::open(&path).unwrap();
    interactive.ensure_local_host("local").unwrap();

    for _ in 0..4 {
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let registry =
                HostRegistry::open_existing_interruptible(worker_path, || false).unwrap();
            for _ in 0..20 {
                assert!(std::process::Command::new("true")
                    .status()
                    .unwrap()
                    .success());
                registry.snapshot().unwrap();
            }
        })
        .join()
        .unwrap();
        interactive.snapshot().unwrap();
    }
}

#[test]
fn an_idle_second_connection_survives_cross_thread_process_spawns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut interactive = HostRegistry::open(&path).unwrap();
    interactive.ensure_local_host("local").unwrap();
    let worker_registry = HostRegistry::open_existing_interruptible(&path, || false).unwrap();

    let worker = std::thread::spawn(move || {
        for _ in 0..40 {
            assert!(std::process::Command::new("true")
                .status()
                .unwrap()
                .success());
        }
        worker_registry.snapshot()
    });

    worker.join().unwrap().unwrap();
    interactive.snapshot().unwrap();
}

#[test]
fn a_second_connection_keeps_wal_readable_after_the_primary_closes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut primary = HostRegistry::open(&path).unwrap();
    primary.ensure_local_host("local").unwrap();
    let worker = HostRegistry::open_existing_interruptible(&path, || false).unwrap();

    drop(primary);

    assert_eq!(worker.journal_mode().unwrap(), "wal");
    worker.snapshot().unwrap();
}

#[test]
fn transient_connections_never_replace_an_active_wal_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let mut primary = HostRegistry::open(&path).unwrap();
    primary.ensure_local_host("local").unwrap();
    primary.snapshot().unwrap();
    let wal = path.with_extension("sqlite3-wal");
    let shm = path.with_extension("sqlite3-shm");
    let wal_inode = fs::metadata(&wal).unwrap().ino();
    let shm_inode = fs::metadata(&shm).unwrap().ino();

    for _ in 0..4 {
        let transient = HostRegistry::open_existing_interruptible(&path, || false).unwrap();
        transient.snapshot().unwrap();
        drop(transient);
        assert_eq!(fs::metadata(&wal).unwrap().ino(), wal_inode);
        assert_eq!(fs::metadata(&shm).unwrap().ino(), shm_inode);
        primary.snapshot().unwrap();
    }
}
