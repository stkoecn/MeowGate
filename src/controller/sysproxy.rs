use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemProxyStatus {
    pub supported: bool,
    pub enabled: bool,
    pub server: Option<String>,
    pub platform: String,
    pub details: Option<String>,
}

#[cfg(windows)]
pub fn get_system_proxy_status(port: u16) -> SystemProxyStatus {
    let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
    let enable_output = std::process::Command::new("reg.exe")
        .args(["query", key, "/v", "ProxyEnable"])
        .output();

    let is_enabled = match enable_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains("0x1") || text.lines().any(|l| l.contains("ProxyEnable") && l.ends_with('1'))
        }
        _ => false,
    };

    let server_output = std::process::Command::new("reg.exe")
        .args(["query", key, "/v", "ProxyServer"])
        .output();

    let server = match server_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .find(|l| l.contains("ProxyServer"))
                .and_then(|l| l.split_whitespace().last())
                .map(|s| s.to_string())
        }
        _ => None,
    };

    let target = format!("127.0.0.1:{}", port);
    let effective_server = server.or(Some(target));

    SystemProxyStatus {
        supported: true,
        enabled: is_enabled,
        server: effective_server,
        platform: "windows".to_string(),
        details: Some("Windows WinINet Internet Settings (HKCU)".to_string()),
    }
}

#[cfg(windows)]
pub fn set_system_proxy(port: u16, enable: bool) -> Result<SystemProxyStatus, String> {
    let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
    let target = format!("127.0.0.1:{}", port);
    let bypass = "<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*";

    if enable {
        let s1 = std::process::Command::new("reg.exe")
            .args(["add", key, "/v", "ProxyEnable", "/t", "REG_DWORD", "/d", "1", "/f"])
            .output()
            .map_err(|e| format!("Failed to set ProxyEnable: {}", e))?;
        if !s1.status.success() {
            return Err("Failed to execute reg.exe for ProxyEnable".to_string());
        }

        let _ = std::process::Command::new("reg.exe")
            .args(["add", key, "/v", "ProxyServer", "/t", "REG_SZ", "/d", &target, "/f"])
            .output();

        let _ = std::process::Command::new("reg.exe")
            .args(["add", key, "/v", "ProxyOverride", "/t", "REG_SZ", "/d", bypass, "/f"])
            .output();
    } else {
        let s1 = std::process::Command::new("reg.exe")
            .args(["add", key, "/v", "ProxyEnable", "/t", "REG_DWORD", "/d", "0", "/f"])
            .output()
            .map_err(|e| format!("Failed to disable ProxyEnable: {}", e))?;
        if !s1.status.success() {
            return Err("Failed to execute reg.exe to disable ProxyEnable".to_string());
        }
    }

    refresh_windows_internet_options();
    Ok(get_system_proxy_status(port))
}

#[cfg(windows)]
fn refresh_windows_internet_options() {
    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            h_internet: *mut std::ffi::c_void,
            dw_option: u32,
            lp_buffer: *mut std::ffi::c_void,
            dw_buffer_length: u32,
        ) -> i32;
    }

    unsafe {
        // INTERNET_OPTION_SETTINGS_CHANGED = 39
        InternetSetOptionW(std::ptr::null_mut(), 39, std::ptr::null_mut(), 0);
        // INTERNET_OPTION_REFRESH = 37
        InternetSetOptionW(std::ptr::null_mut(), 37, std::ptr::null_mut(), 0);
    }
}

#[cfg(not(windows))]
pub fn get_system_proxy_status(port: u16) -> SystemProxyStatus {
    let target = format!("127.0.0.1:{}", port);

    // 1. Check GNOME Desktop (gsettings)
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
    {
        if output.status.success() {
            let mode = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if mode == "'manual'" || mode == "manual" {
                return SystemProxyStatus {
                    supported: true,
                    enabled: true,
                    server: Some(target),
                    platform: "linux".to_string(),
                    details: Some("GNOME Desktop (gsettings)".to_string()),
                };
            }
        }
    }

    // 2. Check /etc/profile.d/meowgate-proxy.sh
    let profile_path = std::path::Path::new("/etc/profile.d/meowgate-proxy.sh");
    if profile_path.exists() {
        if let Ok(content) = std::fs::read_to_string(profile_path) {
            if content.contains(&format!(":{}", port)) {
                return SystemProxyStatus {
                    supported: true,
                    enabled: true,
                    server: Some(target),
                    platform: "linux".to_string(),
                    details: Some("System-wide Environment (/etc/profile.d/meowgate-proxy.sh)".to_string()),
                };
            }
        }
    }

    // 3. Check ~/.meowgate-proxy.env
    if let Ok(home) = std::env::var("HOME") {
        let user_env = std::path::PathBuf::from(home).join(".meowgate-proxy.env");
        if user_env.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_env) {
                if content.contains(&format!(":{}", port)) {
                    return SystemProxyStatus {
                        supported: true,
                        enabled: true,
                        server: Some(target),
                        platform: "linux".to_string(),
                        details: Some("User Shell Environment (~/.meowgate-proxy.env)".to_string()),
                    };
                }
            }
        }
    }

    // 4. Check current process/system environment variables
    if let Ok(val) = std::env::var("http_proxy").or_else(|_| std::env::var("HTTP_PROXY")) {
        if val.contains(&format!(":{}", port)) {
            return SystemProxyStatus {
                supported: true,
                enabled: true,
                server: Some(val),
                platform: "linux".to_string(),
                details: Some("Shell Environment (http_proxy)".to_string()),
            };
        }
    }

    SystemProxyStatus {
        supported: true,
        enabled: false,
        server: Some(target),
        platform: "linux".to_string(),
        details: Some("Linux System Proxy (gsettings / profile.d)".to_string()),
    }
}

#[cfg(not(windows))]
pub fn set_system_proxy(port: u16, enable: bool) -> Result<SystemProxyStatus, String> {
    let expected_http = format!("http://127.0.0.1:{}", port);
    let expected_socks = format!("socks5://127.0.0.1:{}", port);

    // 1. GNOME Desktop (gsettings)
    if let Ok(which_out) = std::process::Command::new("which").arg("gsettings").output() {
        if which_out.status.success() {
            if enable {
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy", "mode", "manual"])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.http", "host", "127.0.0.1"])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.http", "port", &port.to_string()])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.https", "host", "127.0.0.1"])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.https", "port", &port.to_string()])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.socks", "host", "127.0.0.1"])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy.socks", "port", &port.to_string()])
                    .output();
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy", "ignore-hosts", "['localhost', '127.0.0.0/8', '10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16', '::1']"])
                    .output();
            } else {
                let _ = std::process::Command::new("gsettings")
                    .args(["set", "org.gnome.system.proxy", "mode", "none"])
                    .output();
            }
        }
    }

    // 2. Profile script (/etc/profile.d or user profile)
    let env_content = format!(
        "export http_proxy=\"{http}\"\n\
         export https_proxy=\"{http}\"\n\
         export HTTP_PROXY=\"{http}\"\n\
         export HTTPS_PROXY=\"{http}\"\n\
         export all_proxy=\"{socks}\"\n\
         export ALL_PROXY=\"{socks}\"\n\
         export no_proxy=\"localhost,127.0.0.1,localaddress,.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,::1\"\n\
         export NO_PROXY=\"localhost,127.0.0.1,localaddress,.local,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,::1\"\n",
        http = expected_http,
        socks = expected_socks
    );

    let system_profile = std::path::Path::new("/etc/profile.d/meowgate-proxy.sh");
    if enable {
        if let Err(_) = std::fs::write(system_profile, &env_content) {
            if let Ok(home) = std::env::var("HOME") {
                let user_env = std::path::PathBuf::from(home).join(".meowgate-proxy.env");
                let _ = std::fs::write(&user_env, &env_content);
            }
        }
    } else {
        if system_profile.exists() {
            let _ = std::fs::remove_file(system_profile);
        }
        if let Ok(home) = std::env::var("HOME") {
            let user_env = std::path::PathBuf::from(home).join(".meowgate-proxy.env");
            if user_env.exists() {
                let _ = std::fs::remove_file(user_env);
            }
        }
    }

    Ok(get_system_proxy_status(port))
}
