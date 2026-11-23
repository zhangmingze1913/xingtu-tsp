//! Retention policy for PII vault and key audit records.

use chrono::{Duration, Utc};

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub pii_days: i64,
    pub key_audit_days: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            pii_days: 365,
            key_audit_days: 730,
        }
    }
}

impl RetentionPolicy {
    pub fn pii_cutoff(&self) -> String {
        (Utc::now() - Duration::days(self.pii_days.max(1))).to_rfc3339()
    }

    pub fn key_audit_cutoff(&self) -> String {
        (Utc::now() - Duration::days(self.key_audit_days.max(1))).to_rfc3339()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoffs_are_before_now_and_positive() {
        let policy = RetentionPolicy::default();
        let now = Utc::now();
        let pii = chrono::DateTime::parse_from_rfc3339(&policy.pii_cutoff()).unwrap();
        let audit = chrono::DateTime::parse_from_rfc3339(&policy.key_audit_cutoff()).unwrap();
        assert!(pii < now);
        assert!(audit < now);
        assert!(pii > audit);
    }
}
