//! Field-level access policy for personal/sensitive data.

use super::pii::{classify_key, mask_for_key, DataClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Operator,
    Viewer,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "admin" => Some(Self::Admin),
            "operator" | "dispatcher" => Some(Self::Operator),
            "viewer" | "readonly" => Some(Self::Viewer),
            _ => None,
        }
    }
}

pub fn can_view(role: Role, class: DataClass) -> bool {
    match (role, class) {
        (Role::Admin, _) => true,
        (Role::Operator, DataClass::Public | DataClass::Internal | DataClass::Personal) => true,
        (Role::Operator, DataClass::Sensitive) => false,
        (Role::Viewer, DataClass::Public | DataClass::Internal) => true,
        (Role::Viewer, DataClass::Personal | DataClass::Sensitive) => false,
    }
}

/// Whether a role may see the raw value for a field key.
pub fn can_view_key(role: Role, key: &str) -> bool {
    can_view(role, classify_key(key))
}

/// Return the visible value for a role, masking when access is denied.
pub fn visible_value(role: Role, key: &str, value: &str) -> String {
    if can_view(role, classify_key(key)) {
        value.to_string()
    } else {
        mask_for_key(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_policy_masks_by_classification() {
        assert_eq!(
            visible_value(Role::Admin, "phone", "13800138000"),
            "13800138000"
        );
        assert_eq!(
            visible_value(Role::Operator, "phone", "13800138000"),
            "13800138000"
        );
        assert_eq!(
            visible_value(Role::Operator, "id_card", "110101199001011234"),
            "110***********1234"
        );
        assert_eq!(
            visible_value(Role::Viewer, "vin", "LSVAA123456789012"),
            "LSV**********9012"
        );
        assert_eq!(visible_value(Role::Viewer, "speed", "50"), "50");
        assert!(Role::parse("dispatcher").is_some());
        assert!(Role::parse("unknown").is_none());
    }
}
