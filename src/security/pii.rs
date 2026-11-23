//! Minimal PII/vehicle identifier masking for logs and exports.

use sha2::{Digest, Sha256};

fn mask_middle(value: &str, keep_front: usize, keep_back: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= keep_front + keep_back {
        return "*".repeat(chars.len());
    }
    let mut out = String::with_capacity(chars.len());
    out.extend(&chars[..keep_front]);
    out.push_str(&"*".repeat(chars.len() - keep_front - keep_back));
    out.extend(&chars[chars.len() - keep_back..]);
    out
}

pub fn mask_phone(value: &str) -> String {
    mask_middle(value, 3, 4)
}

pub fn mask_vin(value: &str) -> String {
    mask_middle(value, 3, 4)
}

pub fn mask_id_card(value: &str) -> String {
    mask_middle(value, 3, 4)
}

pub fn mask_email(value: &str) -> String {
    match value.split_once('@') {
        Some((local, domain)) if !local.is_empty() && !domain.is_empty() => {
            let first = local.chars().next().unwrap_or('*');
            format!("{first}***@{domain}")
        }
        _ => "*".repeat(value.chars().count()),
    }
}

pub fn mask_plate(value: &str) -> String {
    mask_middle(value, 2, 2)
}

pub fn mask_name(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => format!("{first}**"),
        None => String::new(),
    }
}

/// Data classification for export/logging policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    Public,
    Internal,
    Personal,
    Sensitive,
}

pub fn classify_key(key: &str) -> DataClass {
    match key.to_ascii_lowercase().as_str() {
        "phone" | "mobile" | "email" | "vin" | "plate" | "driver_name" | "owner_name" | "imei"
        | "iccid" => DataClass::Personal,
        "id_card" | "password" | "token" | "secret" | "license_key" => DataClass::Sensitive,
        "latitude" | "longitude" | "speed" | "odometer" => DataClass::Internal,
        _ => DataClass::Public,
    }
}

pub fn should_mask(key: &str) -> bool {
    matches!(
        classify_key(key),
        DataClass::Personal | DataClass::Sensitive
    )
}

/// Recursively mask string fields whose key is classified as personal/sensitive.
pub fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if should_mask(key) {
                    if let serde_json::Value::String(text) = item {
                        *text = mask_for_key(key, text);
                        continue;
                    }
                }
                redact_json(item);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json(item);
            }
        }
        _ => {}
    }
}

pub fn mask_for_key(key: &str, value: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "phone" | "mobile" => mask_phone(value),
        "imei" | "iccid" => mask_middle(value, 3, 4),
        "vin" => mask_vin(value),
        "id_card" => mask_id_card(value),
        "email" => mask_email(value),
        "plate" => mask_plate(value),
        "driver_name" | "owner_name" => mask_name(value),
        _ => "***".to_string(),
    }
}

/// Redact a JSON string; non-JSON text is returned unchanged.
pub fn redact_json_text(text: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut value) => {
            redact_json(&mut value);
            value.to_string()
        }
        Err(_) => text.to_string(),
    }
}

/// Mask 11-digit mainland mobile numbers inside free text.
pub fn mask_phone_in_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_digit() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            let run = &chars[start..index];
            if run.len() == 11 && run[0] == '1' {
                out.extend(&run[..3]);
                out.push_str("****");
                out.extend(&run[7..]);
            } else {
                out.extend(run);
            }
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// JSON-aware redaction plus free-text mobile masking.
pub fn redact_text(text: &str) -> String {
    mask_phone_in_text(&redact_json_text(text))
}

/// Deterministic salted pseudonym for joins/analytics without exposing PII.
/// This is not a password hash and must not be used for credentials.
pub fn pseudonymize(value: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
