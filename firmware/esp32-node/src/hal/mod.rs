//! ESP-IDF implementations of the traits in `rhizo_node_app::ports`.
//!
//! Everything here is an adapter. No decision is made in this module tree: the
//! decisions live in `rhizo-node-app`, which has no ESP-IDF dependency and is
//! tested on the host with fakes.
//!
//! **No GPIO number appears here.** An adapter receives its already-constructed
//! pin and its polarity from `src/board/`; it never names one. That is what
//! makes adding a board a new file in `src/board/` rather than a refactor
//! (F-090-45).

pub mod battery;
pub mod clock;
pub mod nvs;
pub mod pump;
pub mod rail;
pub mod rng;
pub mod rtc_store;
pub mod sleep;
pub mod watchdog;
