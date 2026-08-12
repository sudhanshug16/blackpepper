use super::tests::{write_helper, RecordingTransport};
use super::*;
use std::fs;

#[test]
fn checksum_mismatch_preserves_final_helper_and_removes_upload_temp() {
    let fixture = VerificationFixture::new(crate::BUILD_ID);
    let mut transport = fixture.transport();
    transport.mutate_upload_source = Some(fixture.bundled.clone());

    let error =
        install_bundled_helper(&mut transport, &fixture.managed, &fixture.bundled).unwrap_err();

    fixture.assert_rejected_without_publish(&error);
}

#[test]
fn version_mismatch_is_rejected_before_atomic_publish_and_temp_is_removed() {
    let fixture = VerificationFixture::new("wrong-packaged-build");
    let mut transport = fixture.transport();

    let error =
        install_bundled_helper(&mut transport, &fixture.managed, &fixture.bundled).unwrap_err();

    fixture.assert_rejected_without_publish(&error);
}

struct VerificationFixture {
    root: tempfile::TempDir,
    data_home: PathBuf,
    bin: PathBuf,
    bundled: PathBuf,
    managed: ManagedHelperLocation,
}

impl VerificationFixture {
    fn new(packaged_build: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let data_home = root.path().join("remote-data");
        let directory = helper_install_directory(
            &data_home,
            crate::BUILD_ID,
            SidecarTarget::LinuxX86_64.triple(),
        );
        let final_path = directory.join("bp-host");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&final_path, b"existing final").unwrap();
        let bundled = root.path().join("bundled-bp-host");
        write_helper(&bundled, packaged_build);
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        Self {
            root,
            data_home,
            bin,
            bundled,
            managed: ManagedHelperLocation {
                target: SidecarTarget::LinuxX86_64,
                directory,
                final_path,
            },
        }
    }

    fn transport(&self) -> RecordingTransport {
        RecordingTransport::new(self.root.path(), self.data_home.clone(), &self.bin)
    }

    fn assert_rejected_without_publish(&self, error: &str) {
        assert!(error.contains("checksum or version verification failed"));
        assert_eq!(
            fs::read(&self.managed.final_path).unwrap(),
            b"existing final"
        );
        let entries = fs::read_dir(&self.managed.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(
            entries
                .iter()
                .all(|name| !name.to_string_lossy().ends_with(".upload")),
            "temporary helper upload was not cleaned up: {entries:?}"
        );
    }
}
