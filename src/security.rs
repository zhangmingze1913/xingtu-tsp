//! Security utilities for personal/vehicle data handling.

pub mod access;
pub mod compliance;
pub mod crypto;
pub mod logging;
pub mod pii;
pub mod retention;

#[cfg(test)]
mod tests {
    use super::pii::{
        classify_key, mask_email, mask_id_card, mask_name, mask_phone, mask_phone_in_text,
        mask_plate, mask_vin, pseudonymize, redact_json, redact_json_text, redact_text,
        should_mask, DataClass,
    };

    #[test]
    fn pii_masking_keeps_only_minimum_context() {
        assert_eq!(mask_phone("13800138000"), "138****8000");
        assert_eq!(mask_phone("123"), "***");
        assert_eq!(mask_vin("LSVAA123456789012"), "LSV**********9012");
        assert_eq!(mask_vin("SHORT"), "*****");
        assert_eq!(mask_id_card("110101199001011234"), "110***********1234");
        assert_eq!(mask_id_card("123"), "***");
        assert_eq!(mask_email("alice@example.com"), "a***@example.com");
        assert_eq!(mask_plate("沪A12345"), "沪A***45");
        assert_eq!(mask_name("张三"), "张**");
        assert!(should_mask("vin"));
        assert!(!should_mask("speed"));
        assert_eq!(classify_key("password"), DataClass::Sensitive);

        let mut payload = serde_json::json!({
            "vin": "LSVAA123456789012",
            "nested": {"phone": "13800138000", "speed": 50},
            "token": "secret-token"
        });
        redact_json(&mut payload);
        assert_eq!(payload["vin"], "LSV**********9012");
        assert_eq!(payload["nested"]["phone"], "138****8000");
        assert_eq!(payload["nested"]["speed"], 50);
        assert_eq!(payload["token"], "***");
        let redacted = redact_json_text(r#"{"phone":"13800138000","speed":50}"#);
        assert!(redacted.contains("138****8000"));
        assert!(redacted.contains("\"speed\":50"));
        assert_eq!(redact_json_text("plain text"), "plain text");
        assert_eq!(redact_text("call 13800138000 now"), "call 138****8000 now");
        assert_eq!(mask_phone_in_text("12345678901"), "123****8901");
        let pseudo = pseudonymize("LSVAA123456789012", "org-salt");
        assert_eq!(pseudo.len(), 32);
        assert_eq!(pseudo, pseudonymize("LSVAA123456789012", "org-salt"));
        assert_ne!(pseudo, pseudonymize("LSVAA123456789012", "other-salt"));
    }
}
