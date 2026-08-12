use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) struct Config {
    pub root: PathBuf,
    pub cargo: PathBuf,
    pub strip: PathBuf,
    pub target_dir: PathBuf,
    pub build_output_dir: PathBuf,
    pub bundle_root: PathBuf,
    pub log: PathBuf,
    pub launch_cwd: PathBuf,
    pub host_target: String,
    pub build_prefix: String,
    pub debounce: Duration,
    pub grace: Duration,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let default_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| "development watcher is outside the repository".to_owned())?;
        let root = environment_path("BLACKPEPPER_DEV_WATCH_ROOT")
            .unwrap_or_else(|| default_root.to_path_buf())
            .canonicalize()
            .map_err(|error| format!("development source root is unavailable: {error}"))?;
        let cargo = environment_path("BLACKPEPPER_DEV_WATCH_CARGO")
            .or_else(|| environment_path("CARGO"))
            .unwrap_or_else(|| PathBuf::from("cargo"));
        let strip = environment_path("BLACKPEPPER_DEV_WATCH_STRIP").ok_or_else(|| {
            "BLACKPEPPER_DEV_WATCH_STRIP is unset; run through scripts/dev-watch.sh".to_owned()
        })?;
        let target_dir =
            environment_path("BLACKPEPPER_DEV_TARGET_DIR").unwrap_or_else(|| root.join("target"));
        if !target_dir.is_absolute() {
            return Err(format!(
                "development target directory must be absolute: {}",
                target_dir.display()
            ));
        }
        let host_target = match std::env::var("BLACKPEPPER_DEV_WATCH_HOST_TARGET") {
            Ok(value) if !value.is_empty() => value,
            _ => rust_host_target()?,
        };
        let package_version = package_version(&root)?;
        let session = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before the Unix epoch".to_owned())?
            .as_nanos();
        Ok(Self {
            build_output_dir: target_dir.join(&host_target).join("debug"),
            bundle_root: target_dir.join("blackpepper-dev-watch/bundles"),
            log: environment_path("BLACKPEPPER_DEV_WATCH_LOG")
                .unwrap_or_else(|| target_dir.join("blackpepper-dev-watch.log")),
            launch_cwd: std::env::current_dir()
                .map_err(|error| format!("current working directory is unavailable: {error}"))?,
            build_prefix: format!("{package_version}-watch.{session}.{}", std::process::id()),
            root,
            cargo,
            strip,
            target_dir,
            host_target,
            debounce: positive_duration("BLACKPEPPER_DEV_WATCH_DEBOUNCE", 0.35)?,
            grace: positive_duration("BLACKPEPPER_DEV_WATCH_GRACE", 10.0)?,
        })
    }

    pub fn build_id(&self, sequence: u64) -> String {
        format!("{}-{sequence}", self.build_prefix)
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn rust_host_target() -> Result<String, String> {
    let rustc = environment_path("RUSTC").unwrap_or_else(|| PathBuf::from("rustc"));
    let output = Command::new(&rustc)
        .arg("-vV")
        .output()
        .map_err(|error| format!("could not run {}: {error}", rustc.display()))?;
    if !output.status.success() {
        return Err(format!("{} -vV exited {}", rustc.display(), output.status));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{} -vV did not report a host target", rustc.display()))
}

fn package_version(root: &Path) -> Result<String, String> {
    let manifest_path = root.join("crates/blackpepper/Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
    let parsed = toml::from_str::<toml::Value>(&manifest)
        .map_err(|error| format!("could not parse {}: {error}", manifest_path.display()))?;
    parsed
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .filter(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "{} has no filesystem-safe package.version",
                manifest_path.display()
            )
        })
}

fn positive_duration(name: &str, default: f64) -> Result<Duration, String> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    let seconds = raw
        .parse::<f64>()
        .map_err(|_| format!("{name} must be a number, got {raw:?}"))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(Duration::from_secs_f64(seconds))
}
