use serde_json::Value;

fn production_security() -> Value {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    config["app"]["security"].clone()
}

fn connect_sources(csp: &Value) -> Vec<&str> {
    csp.as_str()
        .unwrap()
        .split(';')
        .find_map(|directive| {
            let mut parts = directive.split_ascii_whitespace();
            (parts.next() == Some("connect-src")).then(|| parts.collect())
        })
        .expect("connect-src must be set")
}

#[test]
fn production_connections_are_limited_to_the_app_and_ipc() {
    let security = production_security();
    assert_eq!(
        connect_sources(&security["csp"]),
        ["'self'", "ipc:", "http://ipc.localhost"]
    );
    assert!(security.get("devCsp").is_none());
}

#[test]
fn development_connections_also_allow_the_local_vite_server() {
    let config: Value = serde_json::from_str(include_str!("../tauri.dev.conf.json")).unwrap();
    let security = &config["app"]["security"];
    assert_eq!(
        connect_sources(&security["devCsp"]),
        [
            "'self'",
            "ipc:",
            "http://ipc.localhost",
            "ws://localhost:1420",
            "http://localhost:1420"
        ]
    );
    assert!(security.get("csp").is_none());
}

#[test]
fn window_capabilities_allow_https_links_and_no_filesystem_plugin_commands() {
    let security = production_security();
    let mut opener_permissions = Vec::new();
    for capability in security["capabilities"].as_array().unwrap() {
        for permission in capability["permissions"].as_array().unwrap() {
            let id = permission
                .as_str()
                .or_else(|| permission["identifier"].as_str())
                .unwrap();
            assert!(!id.starts_with("fs:"));
            if id.starts_with("opener:") {
                opener_permissions.push(permission.clone());
            }
        }
    }
    assert_eq!(
        opener_permissions,
        [
            serde_json::json!("opener:allow-reveal-item-in-dir"),
            serde_json::json!({
                "identifier": "opener:allow-open-url",
                "allow": [{ "url": "https://**" }]
            })
        ]
    );
}
