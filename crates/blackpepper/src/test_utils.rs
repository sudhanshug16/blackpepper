#[cfg(test)]
use std::env;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

#[cfg(test)]
pub struct EnvVarGuard {
    key: &'static str,
    original: Option<String>,
}

#[cfg(test)]
impl EnvVarGuard {
    pub fn set(key: &'static str, value: String) -> Self {
        let original = env::var(key).ok();
        env::set_var(key, value);
        Self { key, original }
    }
}

#[cfg(test)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.original {
            env::set_var(self.key, value);
        } else {
            env::remove_var(self.key);
        }
    }
}
