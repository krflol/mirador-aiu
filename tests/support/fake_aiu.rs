use std::{path::PathBuf, process::Command, sync::OnceLock};

static FAKE: OnceLock<PathBuf> = OnceLock::new();

pub fn fake_aiu() -> PathBuf {
    FAKE.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let directory = root
            .join("target/test-fakes")
            .join(std::process::id().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join(if cfg!(windows) {
            "fake-aiu.exe"
        } else {
            "fake-aiu"
        });
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg(root.join("tests/support/fake_aiu_program.rs"))
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap();
        assert!(status.success(), "fake AIU compilation failed");
        output
    })
    .clone()
}
