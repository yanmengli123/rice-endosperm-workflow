//! Keyring-backed MCP HTTP header and stdio env values.
//!
//! SQLite stores only names and `has_value`. Actual values live in
//! `secrets::Secret` under `mcp_header:{id}:{name}` / `mcp_env:{id}:{name}`.

use super::secrets::Secret;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpSecretKind {
    Header,
    Env,
}

impl McpSecretKind {
    pub fn secret_name(self, connection_id: &str, entry_name: &str) -> String {
        match self {
            Self::Header => header_secret_name(connection_id, entry_name),
            Self::Env => env_secret_name(connection_id, entry_name),
        }
    }
}

pub fn header_secret_name(connection_id: &str, header_name: &str) -> String {
    format!("mcp_header:{connection_id}:{header_name}")
}

pub fn env_secret_name(connection_id: &str, env_name: &str) -> String {
    format!("mcp_env:{connection_id}:{env_name}")
}

/// Incoming add/update slot. Empty/omitted `value` keeps an existing secret
/// unless `clear` is set or the name is dropped from the submitted list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingSecret {
    pub name: String,
    pub value: Option<String>,
    pub clear: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedactedSecret {
    pub name: String,
    pub has_value: bool,
}

impl RedactedSecret {
    pub fn persist_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "has_value": self.has_value,
        })
    }
}

pub fn persist_slots_json(slots: &[RedactedSecret]) -> serde_json::Value {
    serde_json::Value::Array(slots.iter().map(RedactedSecret::persist_json).collect())
}

/// True when a persisted MCP connections blob still carries secret values.
pub fn persist_json_leaks_secret_values(value: &serde_json::Value) -> bool {
    let Some(connections) = value.as_array() else {
        return secret_array_leaks(value);
    };
    connections.iter().any(|connection| {
        let transport = connection.get("transport").unwrap_or(connection);
        secret_array_leaks(&transport["headers"]) || secret_array_leaks(&transport["env"])
    })
}

fn secret_array_leaks(value: &serde_json::Value) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    items.iter().any(|item| match item {
        serde_json::Value::Array(pair) => pair
            .get(1)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        serde_json::Value::Object(map) => map
            .get("value")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        _ => false,
    })
}

pub fn has_secret(kind: McpSecretKind, connection_id: &str, name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && Secret::get(&kind.secret_name(connection_id, name)).is_ok()
}

/// Move non-empty plaintext values into the keyring and return redacted slots.
pub fn migrate_plaintext(
    connection_id: &str,
    kind: McpSecretKind,
    entries: &mut [IncomingSecret],
) -> Result<(Vec<RedactedSecret>, bool), String> {
    let mut changed = false;
    let mut out = Vec::new();
    for entry in entries.iter_mut() {
        let name = entry.name.trim();
        if name.is_empty() {
            if entry
                .value
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                entry.value = None;
                changed = true;
            }
            continue;
        }
        if let Some(value) = entry.value.take() {
            let value = value.trim();
            if !value.is_empty() {
                Secret::set(&kind.secret_name(connection_id, name), value)
                    .map_err(|error| error.to_string())?;
                changed = true;
            }
        }
        let has_value = has_secret(kind, connection_id, name);
        if has_value || !name.is_empty() {
            out.push(RedactedSecret {
                name: name.to_string(),
                has_value,
            });
        }
    }
    Ok((out, changed))
}

/// Apply an editor submission: set / keep / clear, then drop removed names.
pub fn apply_named_secrets(
    connection_id: &str,
    kind: McpSecretKind,
    incoming: &[IncomingSecret],
    previous_names: &[String],
) -> Result<Vec<RedactedSecret>, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    for entry in incoming {
        let name = entry.name.trim();
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.to_string()) {
            kept.retain(|slot: &RedactedSecret| slot.name != name);
        }
        let secret = kind.secret_name(connection_id, name);
        let new_value = entry
            .value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(value) = new_value {
            Secret::set(&secret, value).map_err(|error| error.to_string())?;
            kept.push(RedactedSecret {
                name: name.to_string(),
                has_value: true,
            });
            continue;
        }
        if entry.clear {
            let _ = Secret::delete(&secret);
            continue;
        }
        let has_value = Secret::get(&secret).is_ok();
        if has_value {
            kept.push(RedactedSecret {
                name: name.to_string(),
                has_value: true,
            });
        }
    }
    for previous in previous_names {
        let previous = previous.trim();
        if previous.is_empty() || seen.contains(previous) {
            continue;
        }
        let _ = Secret::delete(&kind.secret_name(connection_id, previous));
    }
    Ok(kept)
}

/// Resolve values for connect/test. Non-empty incoming values win; otherwise
/// the keyring is read. Never writes secrets.
pub fn hydrate_named_secrets(
    connection_id: &str,
    kind: McpSecretKind,
    incoming: &[IncomingSecret],
) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for entry in incoming {
        let name = entry.name.trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        if let Some(value) = entry
            .value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            out.push((name.to_string(), value.to_string()));
            continue;
        }
        if let Ok(value) = Secret::get(&kind.secret_name(connection_id, name)) {
            out.push((name.to_string(), value));
        }
    }
    out
}

pub fn delete_named_secrets(connection_id: &str, kind: McpSecretKind, names: &[String]) {
    for name in names {
        let name = name.trim();
        if !name.is_empty() {
            let _ = Secret::delete(&kind.secret_name(connection_id, name));
        }
    }
}

pub fn refresh_has_value(
    connection_id: &str,
    kind: McpSecretKind,
    names: &[String],
) -> Vec<RedactedSecret> {
    names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| RedactedSecret {
            name: name.to_string(),
            has_value: has_secret(kind, connection_id, name),
        })
        .collect()
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use serde_json::json;

    fn unique_id() -> String {
        format!("test-{}", uuid::Uuid::new_v4())
    }

    fn incoming(name: &str, value: Option<&str>, clear: bool) -> IncomingSecret {
        IncomingSecret {
            name: name.into(),
            value: value.map(str::to_string),
            clear,
        }
    }

    #[test]
    fn secret_names_are_namespaced() {
        assert_eq!(
            header_secret_name("c1", "Authorization"),
            "mcp_header:c1:Authorization"
        );
        assert_eq!(env_secret_name("c1", "TOKEN"), "mcp_env:c1:TOKEN");
    }

    #[test]
    fn migrate_moves_plaintext_and_rewrites_without_values() {
        let id = unique_id();
        let mut entries = [incoming("Authorization", Some("secret-value"), false)];
        let (redacted, changed) =
            migrate_plaintext(&id, McpSecretKind::Header, &mut entries).unwrap();
        assert!(changed);
        assert_eq!(
            redacted,
            vec![RedactedSecret {
                name: "Authorization".into(),
                has_value: true,
            }]
        );
        assert_eq!(entries[0].value, None);
        assert_eq!(
            Secret::get(&header_secret_name(&id, "Authorization")).unwrap(),
            "secret-value"
        );
        let persisted = persist_slots_json(&redacted);
        assert!(!persist_json_leaks_secret_values(&persisted));
        assert!(!persisted.to_string().contains("secret-value"));
        delete_named_secrets(&id, McpSecretKind::Header, &["Authorization".into()]);
    }

    #[test]
    fn edit_empty_value_keeps_existing_secret() {
        let id = unique_id();
        Secret::set(&header_secret_name(&id, "Authorization"), "secret-value").unwrap();
        let kept = apply_named_secrets(
            &id,
            McpSecretKind::Header,
            &[incoming("Authorization", Some(""), false)],
            &["Authorization".into()],
        )
        .unwrap();
        assert_eq!(
            kept,
            vec![RedactedSecret {
                name: "Authorization".into(),
                has_value: true,
            }]
        );
        assert_eq!(
            Secret::get(&header_secret_name(&id, "Authorization")).unwrap(),
            "secret-value"
        );
        delete_named_secrets(&id, McpSecretKind::Header, &["Authorization".into()]);
    }

    #[test]
    fn edit_omitted_value_keeps_existing_secret() {
        let id = unique_id();
        Secret::set(&env_secret_name(&id, "TOKEN"), "secret-value").unwrap();
        let kept = apply_named_secrets(
            &id,
            McpSecretKind::Env,
            &[incoming("TOKEN", None, false)],
            &["TOKEN".into()],
        )
        .unwrap();
        assert_eq!(kept[0].has_value, true);
        assert_eq!(
            Secret::get(&env_secret_name(&id, "TOKEN")).unwrap(),
            "secret-value"
        );
        delete_named_secrets(&id, McpSecretKind::Env, &["TOKEN".into()]);
    }

    #[test]
    fn edit_explicit_clear_deletes_secret() {
        let id = unique_id();
        Secret::set(&header_secret_name(&id, "Authorization"), "secret-value").unwrap();
        let kept = apply_named_secrets(
            &id,
            McpSecretKind::Header,
            &[incoming("Authorization", None, true)],
            &["Authorization".into()],
        )
        .unwrap();
        assert!(kept.is_empty());
        assert!(Secret::get(&header_secret_name(&id, "Authorization")).is_err());
    }

    #[test]
    fn removing_list_entry_deletes_secret() {
        let id = unique_id();
        Secret::set(&env_secret_name(&id, "TOKEN"), "secret-value").unwrap();
        let kept = apply_named_secrets(&id, McpSecretKind::Env, &[], &["TOKEN".into()]).unwrap();
        assert!(kept.is_empty());
        assert!(Secret::get(&env_secret_name(&id, "TOKEN")).is_err());
    }

    #[test]
    fn delete_connection_removes_header_and_env_secrets() {
        let id = unique_id();
        Secret::set(&header_secret_name(&id, "Authorization"), "secret-value").unwrap();
        Secret::set(&env_secret_name(&id, "TOKEN"), "secret-value").unwrap();
        delete_named_secrets(&id, McpSecretKind::Header, &["Authorization".into()]);
        delete_named_secrets(&id, McpSecretKind::Env, &["TOKEN".into()]);
        assert!(Secret::get(&header_secret_name(&id, "Authorization")).is_err());
        assert!(Secret::get(&env_secret_name(&id, "TOKEN")).is_err());
    }

    #[test]
    fn hydrate_prefers_incoming_then_keyring() {
        let id = unique_id();
        Secret::set(&header_secret_name(&id, "Authorization"), "secret-value").unwrap();
        let hydrated = hydrate_named_secrets(
            &id,
            McpSecretKind::Header,
            &[
                incoming("Authorization", Some(""), false),
                incoming("X-Trace", Some("trace-1"), false),
            ],
        );
        assert_eq!(
            hydrated,
            vec![
                ("Authorization".into(), "secret-value".into()),
                ("X-Trace".into(), "trace-1".into()),
            ]
        );
        delete_named_secrets(&id, McpSecretKind::Header, &["Authorization".into()]);
    }

    #[test]
    fn persist_json_detects_legacy_pairs_and_value_fields() {
        assert!(persist_json_leaks_secret_values(&json!([{
            "transport": {
                "headers": [["Authorization", "secret-value"]],
                "env": []
            }
        }])));
        assert!(persist_json_leaks_secret_values(&json!([{
            "transport": {
                "headers": [{"name": "Authorization", "value": "secret-value"}],
                "env": []
            }
        }])));
        assert!(!persist_json_leaks_secret_values(&json!([{
            "transport": {
                "headers": [{"name": "Authorization", "has_value": true}],
                "env": [{"name": "TOKEN", "has_value": true}]
            }
        }])));
    }

    #[test]
    fn new_value_overwrites_stored_secret() {
        let id = unique_id();
        Secret::set(&header_secret_name(&id, "Authorization"), "old-value").unwrap();
        apply_named_secrets(
            &id,
            McpSecretKind::Header,
            &[incoming("Authorization", Some("secret-value"), false)],
            &["Authorization".into()],
        )
        .unwrap();
        assert_eq!(
            Secret::get(&header_secret_name(&id, "Authorization")).unwrap(),
            "secret-value"
        );
        delete_named_secrets(&id, McpSecretKind::Header, &["Authorization".into()]);
    }
}
