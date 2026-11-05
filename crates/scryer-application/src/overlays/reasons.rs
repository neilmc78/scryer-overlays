//! Refusal reasons for poster overlays (a local addition).
//!
//! Kept beside the overlay code rather than in `error_reasons.rs`, so the
//! local addition never touches that upstream file. Same rules: a code names
//! the condition, never the wording, and a released code is never renamed.

/// The poster push server type is neither `silo` nor `jellyfin`.
pub const PUSH_SERVER_KIND_UNKNOWN: &str = "OVERLAY_PUSH_SERVER_KIND_UNKNOWN";
/// The poster push address is not an http or https URL.
pub const PUSH_ADDRESS_INVALID: &str = "OVERLAY_PUSH_ADDRESS_INVALID";
/// Push was turned on without a server address.
pub const PUSH_ADDRESS_REQUIRED: &str = "OVERLAY_PUSH_ADDRESS_REQUIRED";
/// No enabled Jellyfin connection with an API key points at the push
/// address's host.
pub const PUSH_CONNECTION_MISSING: &str = "OVERLAY_PUSH_CONNECTION_MISSING";

/// Every reason owned by this area.
pub const ALL: &[&str] = &[
    PUSH_SERVER_KIND_UNKNOWN,
    PUSH_ADDRESS_INVALID,
    PUSH_ADDRESS_REQUIRED,
    PUSH_CONNECTION_MISSING,
];

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn reasons_are_distinct_area_prefixed_screaming_snake() {
        let mut distinct = std::collections::BTreeSet::new();
        for code in ALL {
            assert!(distinct.insert(*code), "duplicate reason {code}");
            assert!(code.starts_with("OVERLAY_"), "{code}");
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{code}"
            );
        }
    }
}
