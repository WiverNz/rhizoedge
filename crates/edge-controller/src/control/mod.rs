//! The periodic control plane.
//!
//! M5 evaluates and records. M6 extends this into the loop that can move water;
//! until then nothing here holds an MQTT client.
pub mod threshold;
pub mod tick;
