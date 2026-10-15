//! Re-exports the ESP-IDF build environment to cargo.
//!
//! `embuild` writes the include paths, link flags, and `cfg`s that
//! `esp-idf-sys` produced while building ESP-IDF itself. Without this the crate
//! compiles against no framework at all and fails at link time with missing
//! symbols rather than with anything that names the cause.
fn main() {
    embuild::espidf::sysenv::output();
    let Some(cfg) = embuild::espidf::sysenv::cfg_args() else {
        panic!("ESP-IDF configuration is required");
    };
    assert!(
        cfg.get("esp_idf_esp_task_wdt_panic").is_some(),
        "watchdog must reset on timeout; check sdkconfig.defaults"
    );
    assert!(
        cfg.get("esp_idf_esp_task_wdt_init").is_some(),
        "task watchdog must be initialized"
    );
}
