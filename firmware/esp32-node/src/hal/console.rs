//! Pre-network serial provisioning through ESP-IDF's configured console.
//! No pin mapping or credentials live here, and input is never echoed.

use rhizo_node_app::persist::PersistedState;
use rhizo_node_app::ports::{NvsStore, Watchdog};
use rhizo_node_app::provision::{Console, ConsoleResponse, LineBuffer};

/// Waits for provisioning indefinitely on a new board, or for five seconds
/// on a configured board. A successful commit reboots to apply the settings.
/// Runtime has no serial reader; reboot provides the provisioning window.
pub fn before_network(
    state: &mut PersistedState,
    store: &mut impl NvsStore,
    watchdog: &mut impl Watchdog,
) {
    // SAFETY: stdin is the ESP-IDF console; fcntl changes only its read mode.
    let configured = unsafe {
        esp_idf_sys::fcntl(
            esp_idf_sys::STDIN_FILENO as i32,
            esp_idf_sys::F_SETFL as i32,
            esp_idf_sys::O_NONBLOCK as i32,
        )
    };
    if configured < 0 {
        log::error!("cannot configure provisioning console; network not started");
        loop {
            watchdog.feed();
            esp_idf_hal::delay::FreeRtos::delay_ms(100);
        }
    }
    let provisioned =
        state.provisioning.wifi_ssid.is_some() && state.provisioning.mqtt_host.is_some();
    let deadline = super::clock::monotonic_ms().saturating_add(5000);
    let mut editing = false;
    let mut console = Console::open(state);
    let mut line = LineBuffer::default();
    log::info!("serial provisioning ready; input is not echoed; commit persists and reboots");
    loop {
        watchdog.feed();
        let mut byte = 0u8;
        // SAFETY: byte is writable for one byte and stdin is a valid descriptor.
        let count = unsafe {
            esp_idf_sys::read(
                esp_idf_sys::STDIN_FILENO as i32,
                (&mut byte as *mut u8).cast(),
                1,
            )
        };
        if count == 1 {
            editing = true;
            if let Some(text) = line.push(byte) {
                if !text.is_empty() {
                    match console.handle_durable(&text, state, store) {
                        Ok(ConsoleResponse::Committed) => {
                            log::info!("provisioning committed; rebooting");
                            esp_idf_hal::delay::FreeRtos::delay_ms(100);
                            // SAFETY: reset after the durable commit, before networking or actuation.
                            unsafe {
                                esp_idf_sys::esp_restart();
                            }
                        }
                        Ok(ConsoleResponse::Shown(view)) => log::info!("{view}"),
                        Ok(ConsoleResponse::Staged(field)) => {
                            log::info!("provisioning staged: {field}")
                        }
                        Ok(ConsoleResponse::Unknown(_)) | Ok(ConsoleResponse::Refused) => {
                            log::warn!("provisioning command refused")
                        }
                        Err(_) => {
                            log::error!("provisioning commit failed; previous credentials retained")
                        }
                    }
                }
            }
        } else {
            if provisioned && !editing && super::clock::monotonic_ms() >= deadline {
                console.close();
                return;
            }
            esp_idf_hal::delay::FreeRtos::delay_ms(10);
        }
    }
}
