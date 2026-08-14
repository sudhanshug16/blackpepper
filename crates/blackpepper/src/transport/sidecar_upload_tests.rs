use std::path::Path;

use super::*;
use crate::transport::{release_asset, ManagedTool, ReleaseAsset, SidecarTarget};

fn synthetic_archive(asset: &ReleaseAsset) -> &'static ReleaseAsset {
    Box::leak(Box::new(ReleaseAsset {
        trusted_sha256: Some("2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"),
        // License transfer has separate coverage. Keep this fixture focused on
        // binary verification and remote path construction.
        license_name: None,
        license_sha256: None,
        ..asset.clone()
    }))
}

#[test]
fn upload_plan_is_versioned_and_verifies_before_atomic_move() {
    let asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxAarch64).unwrap();
    // Constructing the token through verification prevents planning from an
    // unverified download. This test asset uses a matching synthetic digest.
    let synthetic = synthetic_archive(asset);
    let verified = synthetic.verify(b"x").unwrap();
    let plan = UploadPlan::new(
        verified,
        "/tmp/zellij",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Path::new("/home/dev"),
    )
    .unwrap();

    assert_eq!(
        plan.remote_binary,
        Path::new(&format!(
            "/home/dev/.local/share/blackpepper/sidecars/zellij/{}/aarch64-unknown-linux-musl/zellij",
            asset.version
        ))
    );
    let command = plan.verify_and_commit_command();
    assert_eq!(command.program, "sh");
    assert!(command.args[1].contains("sha256sum"));
    assert!(command.args[1].contains("exit 74"));
    assert!(command.args[1].contains("mv -f"));

    let custom = UploadPlan::new_in_data_home(
        verified,
        "/tmp/zellij",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Path::new("/srv/blackpepper-data"),
    )
    .unwrap();
    assert_eq!(
        custom.remote_binary,
        Path::new(&format!(
            "/srv/blackpepper-data/blackpepper/sidecars/zellij/{}/aarch64-unknown-linux-musl/zellij",
            asset.version
        ))
    );
    assert!(custom
        .prepare_command()
        .args
        .contains(&"/srv/blackpepper-data/blackpepper".to_string()));
}

#[test]
fn upload_plan_rejects_macos_asset_for_linux_remote() {
    let asset = release_asset(ManagedTool::Zellij, SidecarTarget::MacOsAarch64).unwrap();
    let synthetic = synthetic_archive(asset);
    let verified = synthetic.verify(b"x").unwrap();
    assert!(UploadPlan::new(
        verified,
        "/tmp/zellij",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Path::new("/Users/dev"),
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn upload_plan_rejects_non_utf8_remote_paths() {
    use std::os::unix::ffi::OsStrExt;

    let asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxAarch64).unwrap();
    let synthetic = synthetic_archive(asset);
    let verified = synthetic.verify(b"x").unwrap();
    let remote_home = Path::new(std::ffi::OsStr::from_bytes(b"/home/\xff"));
    assert!(UploadPlan::new(
        verified,
        "/tmp/zellij",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        remote_home,
    )
    .is_err());
}
