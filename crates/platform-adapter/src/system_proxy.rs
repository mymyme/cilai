//! Read-only system proxy settings. No environment-proxy or machine-wide fallback.
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemProxySource {
    MacOsSystemConfiguration,
    WindowsCurrentUser,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemProxyFeature {
    Pac,
    AutoDiscovery,
    Scoped,
    SimpleHostnames,
    Authentication,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemProxySnapshot {
    pub source: SystemProxySource,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub socks_proxy: Option<String>,
    pub bypass: Vec<String>,
    pub unsupported_features: Vec<SystemProxyFeature>,
}

/// Read the current interactive user's system settings without resolving PAC,
/// contacting a proxy, reading credentials, or changing any setting.
/// # Errors
/// Failed, malformed, authenticated, or unsupported platform settings are explicit.
pub fn snapshot() -> Result<SystemProxySnapshot, String> {
    platform::snapshot()
}

fn unauthenticated(value: &str) -> Result<(), String> {
    if value.contains('@') {
        return Err("model_proxy_auth_not_supported".into());
    }
    if value.chars().any(char::is_control) {
        return Err("model_system_proxy_invalid".into());
    }
    Ok(())
}

#[cfg(any(test, target_os = "macos"))]
mod macos_parser {
    use super::{SystemProxyFeature, SystemProxySnapshot, SystemProxySource, unauthenticated};
    use std::collections::BTreeMap;

    enum Value {
        Scalar(String),
        Dictionary(BTreeMap<String, Value>),
        Array(Vec<Value>),
    }
    type Dictionary = BTreeMap<String, Value>;
    const INVALID: &str = "model_system_proxy_invalid";

    pub(super) fn parse(text: &str) -> Result<SystemProxySnapshot, String> {
        if text.len() > 262_144 {
            return Err(INVALID.into());
        }
        let lines: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        if lines.first() != Some(&"<dictionary> {") {
            return Err(INVALID.into());
        }
        let mut position = 1;
        let Value::Dictionary(dictionary) = container(&lines, &mut position, false, 0)? else {
            return Err(INVALID.into());
        };
        if position != lines.len() {
            return Err(INVALID.into());
        }
        let mut result = settings(&dictionary)?;
        if let Some(value) = dictionary.get("__SCOPED__") {
            let Value::Dictionary(scopes) = value else {
                return Err(INVALID.into());
            };
            for scope in scopes.values() {
                let Value::Dictionary(scope) = scope else {
                    return Err(INVALID.into());
                };
                if settings(scope)? != result {
                    result.unsupported_features.push(SystemProxyFeature::Scoped);
                    break;
                }
            }
        }
        for key in [
            "__SERVICES__",
            "__SUPPLEMENTAL__",
            "SupplementalMatchDomains",
        ] {
            if dictionary.get(key).is_some_and(nonempty) {
                result.unsupported_features.push(SystemProxyFeature::Scoped);
            }
        }
        Ok(result)
    }

    fn container(
        lines: &[&str],
        position: &mut usize,
        array: bool,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 16 {
            return Err(INVALID.into());
        }
        let mut dictionary = Dictionary::new();
        let mut values = Vec::new();
        while let Some(line) = lines.get(*position) {
            *position += 1;
            if *line == "}" {
                return Ok(if array {
                    Value::Array(values)
                } else {
                    Value::Dictionary(dictionary)
                });
            }
            let (key, value) = line.split_once(" : ").ok_or(INVALID)?;
            let value = match value {
                "<dictionary> {" => container(lines, position, false, depth + 1)?,
                "<array> {" => container(lines, position, true, depth + 1)?,
                value if !value.contains(['{', '}']) => Value::Scalar(value.to_owned()),
                _ => return Err(INVALID.into()),
            };
            if array {
                if key.parse::<usize>().ok() != Some(values.len()) {
                    return Err(INVALID.into());
                }
                values.push(value);
            } else if key.is_empty() || dictionary.insert(key.to_owned(), value).is_some() {
                return Err(INVALID.into());
            }
        }
        Err(INVALID.into())
    }

    fn nonempty(value: &Value) -> bool {
        match value {
            Value::Scalar(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            Value::Dictionary(value) => !value.is_empty(),
        }
    }
    fn scalar<'a>(dictionary: &'a Dictionary, key: &str) -> Result<Option<&'a str>, String> {
        match dictionary.get(key) {
            Some(Value::Scalar(value)) => Ok(Some(value.as_str())),
            None => Ok(None),
            _ => Err(INVALID.into()),
        }
    }
    fn enabled(dictionary: &Dictionary, key: &str) -> Result<bool, String> {
        match scalar(dictionary, key)? {
            Some("1") => Ok(true),
            Some("0") | None => Ok(false),
            _ => Err(INVALID.into()),
        }
    }
    fn endpoint(dictionary: &Dictionary, prefix: &str) -> Result<Option<String>, String> {
        if !enabled(dictionary, &format!("{prefix}Enable"))? {
            return Ok(None);
        }
        let host = scalar(dictionary, &format!("{prefix}Proxy"))?.ok_or(INVALID)?;
        unauthenticated(host)?;
        if host.is_empty()
            || host.contains(['/', '?', '#'])
            || host.chars().any(char::is_whitespace)
        {
            return Err(INVALID.into());
        }
        let port = scalar(dictionary, &format!("{prefix}Port"))?
            .ok_or(INVALID)?
            .parse::<u16>()
            .map_err(|_| INVALID)?;
        if port == 0 {
            return Err(INVALID.into());
        }
        let host = match host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(ip)) => format!("[{ip}]"),
            _ if host.contains(':') => return Err(INVALID.into()),
            _ => host.to_owned(),
        };
        let scheme = if prefix == "SOCKS" { "socks5" } else { "http" };
        Ok(Some(format!("{scheme}://{host}:{port}")))
    }
    fn settings(dictionary: &Dictionary) -> Result<SystemProxySnapshot, String> {
        let mut unsupported_features = Vec::new();
        for (key, feature) in [
            ("ProxyAutoConfigEnable", SystemProxyFeature::Pac),
            (
                "ProxyAutoDiscoveryEnable",
                SystemProxyFeature::AutoDiscovery,
            ),
            (
                "ExcludeSimpleHostnames",
                SystemProxyFeature::SimpleHostnames,
            ),
        ] {
            if enabled(dictionary, key)? {
                unsupported_features.push(feature);
            }
        }
        for prefix in ["HTTP", "HTTPS", "SOCKS"] {
            if enabled(dictionary, &format!("{prefix}Enable"))?
                && dictionary.keys().any(|key| {
                    key.starts_with(prefix)
                        && ["User", "Password", "Authentication"]
                            .iter()
                            .any(|part| key.contains(part))
                })
            {
                unsupported_features.push(SystemProxyFeature::Authentication);
            }
        }
        let bypass = match dictionary.get("ExceptionsList") {
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    if let Value::Scalar(value) = value {
                        Ok(value.clone())
                    } else {
                        Err(INVALID.to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            _ => return Err(INVALID.into()),
        };
        Ok(SystemProxySnapshot {
            source: SystemProxySource::MacOsSystemConfiguration,
            http_proxy: endpoint(dictionary, "HTTP")?,
            https_proxy: endpoint(dictionary, "HTTPS")?,
            socks_proxy: endpoint(dictionary, "SOCKS")?,
            bypass,
            unsupported_features,
        })
    }
}

#[cfg(any(test, windows))]
mod windows_parser {
    use super::{SystemProxyFeature, SystemProxySnapshot, SystemProxySource, unauthenticated};
    pub(super) fn parse(
        proxy: Option<&str>,
        bypass: Option<&str>,
        pac: bool,
        auto_detect: bool,
    ) -> Result<SystemProxySnapshot, String> {
        let mut result = SystemProxySnapshot {
            source: SystemProxySource::WindowsCurrentUser,
            http_proxy: None,
            https_proxy: None,
            socks_proxy: None,
            bypass: Vec::new(),
            unsupported_features: Vec::new(),
        };
        if pac {
            result.unsupported_features.push(SystemProxyFeature::Pac);
        }
        if auto_detect {
            result
                .unsupported_features
                .push(SystemProxyFeature::AutoDiscovery);
        }
        if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
            rules(proxy, &mut result)?;
        }
        for entry in bypass
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            unauthenticated(entry)?;
            if entry.eq_ignore_ascii_case("<local>") {
                result
                    .unsupported_features
                    .push(SystemProxyFeature::SimpleHostnames);
            } else {
                result.bypass.push(entry.to_owned());
            }
        }
        Ok(result)
    }
    fn address(value: &str) -> Result<String, String> {
        unauthenticated(value)?;
        if value.is_empty() || value.chars().any(char::is_whitespace) {
            return Err("model_system_proxy_invalid".into());
        }
        Ok(if value.contains("://") {
            value.to_owned()
        } else {
            format!("http://{value}")
        })
    }
    fn rules(proxy: &str, result: &mut SystemProxySnapshot) -> Result<(), String> {
        if !proxy.contains(['=', ';']) {
            let proxy = address(proxy.trim())?;
            result.http_proxy = Some(proxy.clone());
            result.https_proxy = Some(proxy);
            return Ok(());
        }
        for entry in proxy
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let (scheme, value) = entry.split_once('=').ok_or("model_system_proxy_invalid")?;
            let target = match scheme.trim().to_ascii_lowercase().as_str() {
                "http" => &mut result.http_proxy,
                "https" => &mut result.https_proxy,
                "socks" => &mut result.socks_proxy,
                "ftp" => {
                    address(value.trim())?;
                    continue;
                }
                _ => return Err("model_system_proxy_protocol_not_supported".into()),
            };
            if target.is_some() {
                return Err("model_system_proxy_invalid".into());
            }
            *target = Some(address(value.trim())?);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SystemProxySnapshot;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    pub(super) fn snapshot() -> Result<SystemProxySnapshot, String> {
        let mut child = Command::new("/usr/sbin/scutil")
            .args(["-d", "--proxy"])
            .env_clear()
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "model_system_proxy_read_failed")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("model_system_proxy_read_failed")?;
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            stdout
                .take(262_145)
                .read_to_end(&mut output)
                .map(|_| output)
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let output = reader
            .join()
            .map_err(|_| "model_system_proxy_read_failed")?
            .map_err(|_| "model_system_proxy_read_failed")?;
        if !status.is_some_and(|status| status.success()) {
            return Err("model_system_proxy_read_failed".into());
        }
        let text = std::str::from_utf8(&output).map_err(|_| "model_system_proxy_invalid")?;
        super::macos_parser::parse(text)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::SystemProxySnapshot;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
    };
    struct OwnedConfig(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG);
    impl Drop for OwnedConfig {
        fn drop(&mut self) {
            for pointer in [
                self.0.lpszAutoConfigUrl,
                self.0.lpszProxy,
                self.0.lpszProxyBypass,
            ] {
                if !pointer.is_null() {
                    // WinHTTP transfers each non-null allocation to its caller; free it exactly once.
                    unsafe {
                        GlobalFree(pointer.cast());
                    }
                }
            }
        }
    }
    fn text(pointer: *const u16) -> Result<Option<String>, String> {
        if pointer.is_null() {
            return Ok(None);
        }
        // A successful WinHTTP result owns a NUL-terminated UTF-16 string until OwnedConfig drops.
        let mut length = 0;
        while length < 32_768 && unsafe { *pointer.add(length) } != 0 {
            length += 1;
        }
        if length == 32_768 {
            return Err("model_system_proxy_invalid".into());
        }
        // The scan above established this readable prefix and its terminating NUL.
        String::from_utf16(unsafe { std::slice::from_raw_parts(pointer, length) })
            .map(Some)
            .map_err(|_| "model_system_proxy_invalid".into())
    }
    pub(super) fn snapshot() -> Result<SystemProxySnapshot, String> {
        let mut config = OwnedConfig(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default());
        // The output struct is initialized; this read-only API uses the current interactive identity.
        if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&raw mut config.0) } == 0 {
            return Err("model_system_proxy_read_failed".into());
        }
        let pac = text(config.0.lpszAutoConfigUrl)?.is_some_and(|value| !value.is_empty());
        let proxy = text(config.0.lpszProxy)?;
        let bypass = text(config.0.lpszProxyBypass)?;
        super::windows_parser::parse(
            proxy.as_deref(),
            bypass.as_deref(),
            pac,
            config.0.fAutoDetect != 0,
        )
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "android",
    target_os = "ios"
))]
mod platform {
    pub(super) fn snapshot() -> Result<super::SystemProxySnapshot, String> {
        Err("model_system_proxy_platform_not_supported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_manual_proxy_and_direct_snapshots_require_closed_valid_structure() {
        let manual = macos_parser::parse("<dictionary> {\n HTTPEnable : 1\n HTTPProxy : 127.0.0.1\n HTTPPort : 7890\n HTTPSNone : ignored\n ExceptionsList : <array> {\n 0 : 127.0.0.1\n }\n}").unwrap();
        assert_eq!(manual.http_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(manual.https_proxy.is_none());
        assert_eq!(manual.bypass, ["127.0.0.1"]);
        assert!(
            macos_parser::parse("<dictionary> {\n}")
                .unwrap()
                .http_proxy
                .is_none()
        );
        for malformed in [
            "",
            "<dictionary> {",
            "<dictionary> {\n HTTPEnable : 1\n}",
            "<dictionary> {\n}\nnoise",
            "<dictionary> {\n HTTPEnable : 0\n HTTPEnable : 1\n}",
        ] {
            assert!(macos_parser::parse(malformed).is_err());
        }
    }
    #[test]
    fn macos_marks_pac_discovery_authentication_and_different_scopes() {
        let snapshot = macos_parser::parse("<dictionary> {\n ProxyAutoConfigEnable : 1\n ProxyAutoConfigURLString : https://example.invalid/secret-pac-token\n ProxyAutoDiscoveryEnable : 1\n ExcludeSimpleHostnames : 1\n HTTPEnable : 1\n HTTPProxy : proxy.invalid\n HTTPPort : 80\n HTTPUser : opaque-user\n __SCOPED__ : <dictionary> {\n en0 : <dictionary> {\n HTTPEnable : 0\n }\n }\n}").unwrap();
        for feature in [
            SystemProxyFeature::Pac,
            SystemProxyFeature::AutoDiscovery,
            SystemProxyFeature::SimpleHostnames,
            SystemProxyFeature::Authentication,
            SystemProxyFeature::Scoped,
        ] {
            assert!(snapshot.unsupported_features.contains(&feature));
        }
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("secret-pac-token"));
        assert!(!serialized.contains("opaque-user"));
    }
    #[test]
    fn macos_allows_identical_scoped_routes_but_rejects_embedded_credentials() {
        let same = macos_parser::parse("<dictionary> {\n HTTPEnable : 1\n HTTPProxy : 127.0.0.1\n HTTPPort : 80\n __SCOPED__ : <dictionary> {\n en0 : <dictionary> {\n HTTPEnable : 1\n HTTPProxy : 127.0.0.1\n HTTPPort : 80\n }\n }\n}").unwrap();
        assert!(same.unsupported_features.is_empty());
        assert_eq!(macos_parser::parse("<dictionary> {\n HTTPEnable : 1\n HTTPProxy : user:secret@proxy.invalid\n HTTPPort : 80\n}").unwrap_err(), "model_proxy_auth_not_supported");
    }
    #[test]
    fn windows_reads_interactive_user_rules_without_a_winhttp_machine_fallback() {
        let single = windows_parser::parse(Some("proxy.invalid:8080"), None, false, false).unwrap();
        assert_eq!(single.http_proxy, single.https_proxy);
        let scoped = windows_parser::parse(
            Some("http=one.invalid:80;https=two.invalid:81"),
            Some("127.0.0.1;<local>"),
            true,
            true,
        )
        .unwrap();
        assert_eq!(scoped.http_proxy.as_deref(), Some("http://one.invalid:80"));
        assert_eq!(scoped.https_proxy.as_deref(), Some("http://two.invalid:81"));
        assert_eq!(scoped.bypass, ["127.0.0.1"]);
        assert_eq!(
            scoped.unsupported_features,
            [
                SystemProxyFeature::Pac,
                SystemProxyFeature::AutoDiscovery,
                SystemProxyFeature::SimpleHostnames
            ]
        );
        assert!(
            windows_parser::parse(None, None, false, false)
                .unwrap()
                .http_proxy
                .is_none()
        );
    }
    #[test]
    fn windows_rejects_malformed_rules_and_credentials_without_exposing_values() {
        for value in [
            "http=one:80;http=two:80",
            "proxy:80;other:80",
            "unknown=proxy:80",
        ] {
            assert!(windows_parser::parse(Some(value), None, false, false).is_err());
        }
        let error = windows_parser::parse(
            Some("http://user:secret@proxy.invalid:80"),
            None,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error, "model_proxy_auth_not_supported");
        assert!(!error.contains("secret"));
    }
}
