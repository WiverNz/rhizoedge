//! Networking. Wi-Fi association and the MQTT session, and nothing else.
//!
//! No decision about watering, configuration, or policy is made here. The
//! wall clock arrives over this connection but its *rules* live in the shared
//! contract crate (`TimeSyncState`), because a second copy on the device is how
//! a device comes to claim synchronisation it does not have.

pub mod mqtt;
pub mod session;
pub mod wifi;
