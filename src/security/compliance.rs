//! Code-side compliance control snapshot. This is evidence for an assessment,
//! not a certification.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Control {
    pub id: &'static str,
    pub title: &'static str,
    pub status: &'static str,
    pub evidence: &'static str,
}

pub fn snapshot() -> Vec<Control> {
    vec![
        Control {
            id: "PII-01",
            title: "PII masking",
            status: "implemented",
            evidence: "security::pii mask_phone/vin/id_card/email/plate/name",
        },
        Control {
            id: "PII-02",
            title: "Field-level access policy",
            status: "implemented",
            evidence: "security::access admin/operator/viewer",
        },
        Control {
            id: "PII-03",
            title: "JSON and free-text redaction",
            status: "implemented",
            evidence: "redact_json/redact_text + RedactingWriter",
        },
        Control {
            id: "PII-04",
            title: "Export and access audit",
            status: "implemented",
            evidence: "vehicle/terminal/alert/trip/trajectory export audit",
        },
        Control {
            id: "CRYPTO-01",
            title: "AES-256-GCM field encryption",
            status: "implemented",
            evidence: "Aes256Gcm envelope with nonce",
        },
        Control {
            id: "CRYPTO-02",
            title: "Key rotation and audit",
            status: "implemented",
            evidence: "KeyRing previous keys + key_audit",
        },
        Control {
            id: "CRYPTO-03",
            title: "Encrypted PII vault",
            status: "implemented",
            evidence: "pii_vault and auth_code/driver fields",
        },
        Control {
            id: "CRYPTO-04",
            title: "KMS/HSM integration",
            status: "partial",
            evidence: "HttpKmsTransport + provider_from_env; no cloud SDK/HSM certification",
        },
        Control {
            id: "RET-01",
            title: "PII retention and purge",
            status: "implemented",
            evidence: "RetentionPolicy + 6h sweeper",
        },
        Control {
            id: "ACC-01",
            title: "GB/T 32960 device admission",
            status: "implemented",
            evidence: "mTLS client CA + VIN allowlist + device registry + per-VIN token bucket",
        },
        Control {
            id: "ACC-02",
            title: "Device admission change audit",
            status: "implemented",
            evidence: "gb32960.device_update audit with operator, VIN, enable/disable, note",
        },
        Control {
            id: "CRYPTO-05",
            title: "SM2/SM3/SM4 protocol primitives",
            status: "implemented",
            evidence: "GB/T 32960 crypto module + standard-vector tests; not a 密评 result",
        },
        Control {
            id: "COMP-01",
            title: "Independent assessment",
            status: "external",
            evidence: "requires third-party assessment and real-device testing",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_marks_external_and_partial_controls_honestly() {
        let controls = snapshot();
        assert!(controls.iter().any(|control| control.id == "CRYPTO-01"));
        assert!(controls.iter().any(|control| control.status == "partial"));
        assert!(controls.iter().any(|control| control.status == "external"));
        assert!(!controls.iter().any(|control| control.status == "certified"));
    }
}
