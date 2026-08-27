//! Typed validation diagnostics.
use alloc::{string::String, vec::Vec};
/// Non-fatal invalid fields in a partially usable payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidationReport {
    /** Invalid field names. */
    pub invalid_fields: Vec<String>,
}
impl ValidationReport {
    /** Returns true when all fields were valid. */
    pub fn is_valid(&self) -> bool {
        self.invalid_fields.is_empty()
    }
    /** Adds an invalid field. */
    pub fn push(&mut self, field: &str) {
        self.invalid_fields.push(String::from(field));
    }
}
