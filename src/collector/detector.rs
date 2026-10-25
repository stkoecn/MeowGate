use std::path::{Path, PathBuf};
use sysinfo::{ProcessesToUpdate, System};
use tracing::{debug, info};
use walkdir::WalkDir;
use crate::collector::yaml_parser::YamlParser;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredProfileInfo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub node_count: usize,
}

/// Inspects running Clash/Mihomo processes and known application data directories
/// to discover all local Clash configuration and profile files (both active and inactive).
pub struct ClashDetector;

impl ClashDetector {
    pub fn discover_all_configs() -> Vec<PathBuf> {
        Self::discover_all_configs_with_custom(&[])
    }

    /// Detects all potential Clash configuration files on the system, including any custom directories:
    /// 1. Inspects command-line arguments of running Clash processes (-f, -d, etc.)
    /// 2. Scans standard app data directories of Clash Verge, Clash for Windows, Clash Nyanpasu, Mihomo Party, etc.
    /// 3. Scans any user-specified custom profile directories.
    pub fn discover_all_configs_with_custom(custom_dirs: &[PathBuf]) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Discover from running processes
        let process_configs = Self::detect_from_running_processes();
        candidates.extend(process_configs);

        // 2. Discover from standard known directories (profiles folder of CFW, Verge, etc.)
        let known_configs = Self::detect_from_known_paths();
        candidates.extend(known_configs);

        // 3. Discover from configured custom directories
        for dir in custom_dirs {
            Self::scan_dir_for_configs(dir, &mut candidates, 3);
        }

        // Deduplicate and filter existing valid files
        let mut valid_files: Vec<PathBuf> = Vec::new();
        for p in candidates {
            if let Ok(canonical) = std::fs::canonicalize(&p) {
                if canonical.is_file() && !valid_files.contains(&canonical) {
                    valid_files.push(canonical);
                }
            } else if p.is_file() && !valid_files.contains(&p) {
                valid_files.push(p);
            }
        }

        info!("Discovered {} local Clash config file(s)", valid_files.len());
        valid_files
    }

    /// Finds running Clash/Mihomo processes and extracts the configuration files or working directories.
    pub fn detect_from_running_processes() -> Vec<PathBuf> {
        let mut configs = Vec::new();
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let target_names = [
            "clash",
            "mihomo",
            "clash-meta",
            "clash-verge",
            "clash-nyanpasu",
            "clash for windows",
        ];

        for (_pid, process) in sys.processes() {
            let proc_name = process.name().to_string_lossy().to_lowercase();
            let is_match = target_names.iter().any(|target| proc_name.contains(target));

            if !is_match {
                continue;
            }

            debug!("Found potential Clash process: {} (PID: {:?})", proc_name, _pid);

            // Check executable directory
            if let Some(exe_path) = process.exe() {
                if let Some(parent) = exe_path.parent() {
                    Self::scan_dir_for_configs(parent, &mut configs, 2);
                }
            }

            // Check current working directory
            if let Some(cwd) = process.cwd() {
                Self::scan_dir_for_configs(cwd, &mut configs, 2);
            }

            // Check CLI arguments for -f <config.yaml> or -d <dir>
            let cmd = process.cmd();
            let mut iter = cmd.iter();
            while let Some(arg) = iter.next() {
                let arg_lossy = arg.to_string_lossy();
                if arg_lossy == "-f" {
                    if let Some(val) = iter.next() {
                        let path = PathBuf::from(val);
                        if path.is_file() {
                            configs.push(path);
                        }
                    }
                } else if arg_lossy == "-d" {
                    if let Some(val) = iter.next() {
                        let dir = PathBuf::from(val);
                        if dir.is_dir() {
                            Self::scan_dir_for_configs(&dir, &mut configs, 2);
                        }
                    }
                } else if arg_lossy.ends_with(".yaml") || arg_lossy.ends_with(".yml") {
                    let path = PathBuf::from(&*arg_lossy);
                    if path.is_file() {
                        configs.push(path);
                    }
                }
            }
        }

        configs
    }

    /// Scans standard app data directories on Windows, macOS, and Linux
    pub fn detect_from_known_paths() -> Vec<PathBuf> {
        let mut configs = Vec::new();

        #[cfg(target_os = "windows")]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                let appdata_path = PathBuf::from(appdata);
                // Clash Verge (rev) profiles
                let verge_profiles = appdata_path.join("clash-verge").join("profiles");
                Self::scan_dir_for_configs(&verge_profiles, &mut configs, 2);

                let verge_rev = appdata_path.join("io.github.clash-verge-rev.clash-verge-rev").join("profiles");
                Self::scan_dir_for_configs(&verge_rev, &mut configs, 2);

                // Clash for Windows profiles
                let cfw_profiles = appdata_path.join("clash_win").join("profiles");
                Self::scan_dir_for_configs(&cfw_profiles, &mut configs, 2);

                // Clash Nyanpasu
                let nyanpasu_profiles = appdata_path.join("clash-nyanpasu").join("profiles");
                Self::scan_dir_for_configs(&nyanpasu_profiles, &mut configs, 2);

                // Mihomo Party
                let mihomo_profiles = appdata_path.join("mihomo-party").join("profiles");
                Self::scan_dir_for_configs(&mihomo_profiles, &mut configs, 2);
            }

            if let Ok(userprofile) = std::env::var("USERPROFILE") {
                let user_path = PathBuf::from(userprofile);
                // ~/.config/clash
                let dot_config_clash = user_path.join(".config").join("clash");
                Self::scan_dir_for_configs(&dot_config_clash, &mut configs, 2);

                // ~/.config/mihomo
                let dot_config_mihomo = user_path.join(".config").join("mihomo");
                Self::scan_dir_for_configs(&dot_config_mihomo, &mut configs, 2);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            if let Ok(home) = std::env::var("HOME") {
                let home_path = PathBuf::from(home);
                let dirs_to_check = [
                    home_path.join(".config/clash"),
                    home_path.join(".config/clash-verge/profiles"),
                    home_path.join(".config/mihomo"),
                    home_path.join(".config/clash-nyanpasu/profiles"),
                    home_path.join("Library/Application Support/clash-verge/profiles"),
                    home_path.join("Library/Application Support/ClashX/config.yaml"),
                    home_path.join("Library/Application Support/io.github.clash-verge-rev.clash-verge-rev/profiles"),
                ];
                for d in dirs_to_check {
                    if d.is_file() {
                        configs.push(d);
                    } else if d.is_dir() {
                        Self::scan_dir_for_configs(&d, &mut configs, 2);
                    }
                }
            }
        }

        configs
    }

    /// Loads profiles metadata from known profiles.yaml files (e.g. Clash Verge, Mihomo Party, Clash Nyanpasu).
    #[allow(dead_code)]
    pub fn load_profiles_metadata() -> std::collections::HashMap<String, (String, String)> {
        Self::load_profiles_metadata_with_custom(&[])
    }

    /// Loads profiles metadata from known profiles.yaml files as well as custom directories.
    pub fn load_profiles_metadata_with_custom(custom_dirs: &[PathBuf]) -> std::collections::HashMap<String, (String, String)> {
        let mut map = std::collections::HashMap::new();

        let mut candidate_files = Vec::new();
        for dir in custom_dirs {
            let p_custom = dir.join("profiles.yaml");
            if p_custom.is_file() {
                candidate_files.push(p_custom);
            }
            if let Some(parent) = dir.parent() {
                let p_parent = parent.join("profiles.yaml");
                if p_parent.is_file() && !candidate_files.contains(&p_parent) {
                    candidate_files.push(p_parent);
                }
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p1 = PathBuf::from(&appdata).join("io.github.clash-verge-rev.clash-verge-rev").join("profiles.yaml");
            let p2 = PathBuf::from(&appdata).join("clash-verge").join("profiles.yaml");
            let p3 = PathBuf::from(&appdata).join("clash-nyanpasu").join("profiles.yaml");
            let p4 = PathBuf::from(&appdata).join("mihomo-party").join("profiles.yaml");
            candidate_files.extend([p1, p2, p3, p4]);
        }
        if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            let p5 = PathBuf::from(&home).join(".config").join("clash-verge").join("profiles.yaml");
            let p6 = PathBuf::from(&home).join(".config").join("clash-nyanpasu").join("profiles.yaml");
            let p7 = PathBuf::from(&home).join("Library").join("Application Support").join("clash-verge").join("profiles.yaml");
            candidate_files.extend([p5, p6, p7]);
        }

        for path in candidate_files {
            if path.exists() && path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                        if let Some(items) = val.get("items").and_then(|i| i.as_sequence()) {
                            for item in items {
                                if let Some(uid) = item.get("uid").and_then(|u| u.as_str()) {
                                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                    let p_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("remote").to_string();
                                    map.insert(uid.to_string(), (name, p_type));
                                }
                            }
                        }
                    }
                }
            }
        }

        map
    }

    #[allow(dead_code)]
    pub fn discover_profile_sources() -> Vec<DiscoveredProfileInfo> {
        Self::discover_profile_sources_with_custom(&[])
    }

    /// Discovers all real individual profile configurations, resolving friendly names.
    /// Never lumps directories together — every valid configuration file is surfaced as its own entity.
    pub fn discover_profile_sources_with_custom(custom_dirs: &[PathBuf]) -> Vec<DiscoveredProfileInfo> {
        let configs = Self::discover_all_configs_with_custom(custom_dirs);
        let meta_map = Self::load_profiles_metadata_with_custom(custom_dirs);

        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for path in configs {
            if !path.exists() || !path.is_file() {
                continue;
            }
            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("config").to_string();
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("config.yaml").to_string();

            if seen.contains(&file_stem) {
                continue;
            }
            seen.insert(file_stem.clone());

            let mut profile_name = String::new();
            if let Some((m_name, p_type)) = meta_map.get(&file_stem) {
                // Skip empty helper rule/merge/script files
                if matches!(p_type.as_str(), "rules" | "script" | "merge" | "groups") {
                    if let Ok(meta) = path.metadata() {
                        if meta.len() == 0 {
                            continue;
                        }
                    }
                }
                if !m_name.is_empty() && m_name != "None" {
                    profile_name = m_name.clone();
                }
            }

            // If no name from metadata, check first comments
            if profile_name.is_empty() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    for line in content.lines().take(5) {
                        let t = line.trim();
                        if let Some(c) = t.strip_prefix('#') {
                            let clean = c.trim();
                            if !clean.is_empty() && clean.len() < 50 && !clean.starts_with("PORT") {
                                profile_name = clean.to_string();
                                break;
                            }
                        }
                    }
                }
            }

            if profile_name.is_empty() {
                profile_name = file_name.clone();
            }

            // Quick check if file has proxies
            let mut count = 0;
            if let Ok(parsed) = YamlParser::parse_file(&path) {
                count = parsed.len();
            }

            // Only include files that actually have proxies or significant size
            let fsize = path.metadata().map(|m| m.len()).unwrap_or(0);
            if count > 0 || fsize > 500 {
                let id = format!("profile-{}", file_stem);
                results.push(DiscoveredProfileInfo {
                    id,
                    name: profile_name,
                    path,
                    node_count: count,
                });
            }
        }

        results
    }

    /// Recursively scans a directory up to max_depth for .yaml / .yml files
    fn scan_dir_for_configs(dir: &Path, out: &mut Vec<PathBuf>, max_depth: usize) {
        if !dir.exists() || !dir.is_dir() {
            return;
        }

        for entry in WalkDir::new(dir)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if ext_lower == "yaml" || ext_lower == "yml" {
                        out.push(path.to_path_buf());
                    }
                }
            }
        }
    }
}
