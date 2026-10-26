use std::process::{Child, Command};
use std::sync::Mutex;
use std::path::{Path, PathBuf};
use tracing::{info, warn, error};

/// Manages the embedded/sub-process proxy core (e.g. verge-mihomo or mihomo/clash)
pub struct CoreManager {
    child: Mutex<Option<Child>>,
}

impl Default for CoreManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreManager {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    /// Finds an available Clash/Mihomo core binary on the machine.
    ///
    /// Prioritizes private isolated binaries in `bin/` (e.g. `bin/meow-core.exe` or `bin/mihomo`)
    /// so MeowGate does NOT run Clash Verge's own `verge-mihomo.exe` directly, avoiding
    /// process ownership and file lock conflicts ("核心进程冲突") with Clash Verge Rev.
    pub fn find_core_binary() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        let primary_isolated = PathBuf::from(r"bin\meow-core.exe");
        #[cfg(not(target_os = "windows"))]
        let primary_isolated = PathBuf::from("bin/meow-core");

        if primary_isolated.exists() && primary_isolated.is_file() {
            return Some(primary_isolated);
        }

        #[allow(unused_mut)]
        let mut candidates = vec![
            PathBuf::from("bin/mihomo"),
            PathBuf::from("bin/clash"),
            PathBuf::from("bin/clash-meta"),
            PathBuf::from("/usr/local/bin/mihomo"),
            PathBuf::from("/usr/bin/mihomo"),
            PathBuf::from("/usr/local/bin/clash-meta"),
            PathBuf::from("/usr/local/bin/clash"),
            PathBuf::from("/opt/mihomo/mihomo"),
        ];

        #[cfg(target_os = "windows")]
        {
            candidates.insert(0, PathBuf::from(r"bin\mihomo.exe"));
            candidates.insert(1, PathBuf::from(r"bin\clash.exe"));
            candidates.insert(2, PathBuf::from(r"bin\verge-mihomo.exe"));
        }

        for c in &candidates {
            if c.exists() && c.is_file() {
                return Some(c.clone());
            }
        }

        // On Windows, if no isolated binary exists yet, check if Clash Verge is installed.
        // If found, copy it to `bin/meow-core.exe` to isolate the process name and executable path,
        // completely preventing "核心进程冲突" with Clash Verge Rev.
        #[cfg(target_os = "windows")]
        {
            let verge_candidates = [
                PathBuf::from(r"D:\Clash Verge\verge-mihomo.exe"),
                PathBuf::from(r"C:\Program Files\Clash Verge\verge-mihomo.exe"),
            ];

            for vc in verge_candidates {
                if vc.exists() && vc.is_file() {
                    let dest = PathBuf::from(r"bin\meow-core.exe");
                    let _ = std::fs::create_dir_all("bin");
                    if std::fs::copy(&vc, &dest).is_ok() {
                        info!("Copied isolated core {:?} -> {:?} to prevent process conflict with Clash Verge", vc, dest);
                        return Some(dest);
                    }
                    return Some(vc);
                }
            }
        }

        // Search PATH environment variable portably
        if let Some(paths) = std::env::var_os("PATH") {
            let names: Vec<&str> = if cfg!(target_os = "windows") {
                vec!["verge-mihomo.exe", "mihomo.exe", "clash.exe", "clash-meta.exe"]
            } else {
                vec!["mihomo", "clash-meta", "clash", "verge-mihomo"]
            };

            for path in std::env::split_paths(&paths) {
                for name in &names {
                    let full = path.join(name);
                    if full.exists() && full.is_file() {
                        return Some(full);
                    }
                }
            }
        }

        None
    }

    /// Starts or restarts the core with the given configuration file path
    pub fn start_or_restart(&self, config_path: &Path) -> bool {
        self.stop();

        let core_exe = match Self::find_core_binary() {
            Some(p) => p,
            None => {
                warn!("No core executable (verge-mihomo.exe) found to start automatically.");
                return false;
            }
        };

        info!("Starting embedded core: {:?} -f {:?}", core_exe, config_path);

        let mut cmd = Command::new(&core_exe);
        cmd.arg("-f").arg(config_path);
        if let Some(parent) = config_path.parent() {
            if !parent.as_os_str().is_empty() {
                cmd.arg("-d").arg(parent);
            }
        }

        match cmd
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("HTTPS_PROXY")
            .env_remove("https_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .spawn()
        {
            Ok(child) => {
                info!("Core started successfully with PID: {:?}", child.id());
                let mut lock = self.child.lock().unwrap();
                *lock = Some(child);
                true
            }
            Err(e) => {
                error!("Failed to spawn core process {:?}: {}", core_exe, e);
                false
            }
        }
    }

    pub fn stop(&self) {
        let mut lock = self.child.lock().unwrap();
        if let Some(mut child) = lock.take() {
            info!("Stopping embedded core PID: {:?}", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for CoreManager {
    fn drop(&mut self) {
        self.stop();
    }
}
