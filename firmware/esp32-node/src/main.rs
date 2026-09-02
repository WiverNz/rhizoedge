//! Rhizo Edge ESP32-C3 plant node.
//!
//! # The first statement is `pump.off()`
//!
//! Before Wi-Fi, before MQTT, before NVS. That ordering **is** the requirement
//! (F-090-30, SAFETY-011), and the reason it is stated as an ordering rather
//! than as "somewhere in initialisation" is that every millisecond before it
//! runs is a millisecond in which a floating pin decides whether water moves.
//!
//! The bootloader window before any Rust runs is not coverable from here. Only
//! the hardware pull-down documented in `src/board/devkitm1.rs` covers it, and
//! HIL-1 is what proves it.
//!
//! # Where the logic is
//!
//! Almost nowhere in this crate. Every safety-relevant decision lives in
//! `rhizo-node-app`, which has no ESP-IDF dependency and is exercised by 111
//! host tests with fake adapters. This file is the loop that wires adapters to
//! that logic; `src/board/` is the only place a GPIO number appears.

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

use rhizo_node_app::ports::{NvsStore, Pump, Watchdog};

mod board;
mod hal;
mod net;
mod run;

fn main() {
    // ESP-IDF's `main` is called from a C runtime that has not run the linker
    // patches Rust's `std` needs.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // ---------------------------------------------------------------- STEP 1
    // The pump, de-energised, before anything that can fail or block.
    //
    // `board::take()` constructs the pump already inactive — `GpioPump::new`
    // drives the inactive level before it returns — so there is no window in
    // which a constructed pump is in an unknown state. `pump.off()` here is
    // belt and braces and costs one register write.
    let mut board = match board::take() {
        Ok(board) => board,
        Err(error) => {
            // A device that cannot construct its pump cannot safely do
            // anything else, so it stops rather than continuing into a state
            // where nothing knows which pin drives what.
            log::error!("board init failed: {error}; halting with the pump undriven");
            halt();
        }
    };
    board.pump.off();
    log::info!(
        "boot: pump de-energised first (board profile {})",
        board::PROFILE
    );

    // ---------------------------------------------------------------- STEP 2
    // Every switched rail. An unexpected reset must not leave a transceiver
    // powered from a battery for two weeks.
    if let Some(rail) = board.sensor_rail.as_mut() {
        rhizo_node_app::ports::PowerRail::disable(rail);
    }
    if let Some(rail) = board.rs485_rail.as_mut() {
        rhizo_node_app::ports::PowerRail::disable(rail);
    }

    // ---------------------------------------------------------------- STEP 3
    // The watchdog, so a hang from here on is a reset that lands in step 1.
    let mut watchdog = match hal::watchdog::TaskWatchdog::subscribe() {
        Ok(watchdog) => watchdog,
        Err(error) => {
            log::error!("watchdog subscribe failed: {error}");
            halt();
        }
    };
    watchdog.feed();
    log::info!("task watchdog subscribed: {}", watchdog.is_subscribed());

    // ---------------------------------------------------------------- STEP 4
    // Why this boot happened, before anything overwrites the latched cause.
    let wake_reason = hal::rtc_store::wake_reason();
    let rtc = hal::rtc_store::read();
    let credited =
        rhizo_node_app::budget::credit_elapsed(wake_reason, Some(&rtc), hal::clock::monotonic_ms());
    log::info!("wake_reason {wake_reason:?}, credited {}ms", credited.0);

    // ---------------------------------------------------------------- STEP 5
    // Persistent state, and the interrupted-dose report that must precede
    // anything network-related.
    let partition = match EspDefaultNvsPartition::take() {
        Ok(partition) => partition,
        Err(error) => {
            log::error!("nvs partition unavailable: {error}");
            halt();
        }
    };
    let mut nvs = match hal::nvs::EspNvsStore::new(partition.clone()) {
        Ok(nvs) => nvs,
        Err(error) => {
            log::error!("nvs namespace unavailable: {error}");
            halt();
        }
    };
    log::info!("nvs active slot {}", nvs.active_slot());
    let mut rng = hal::rng::EspRng;

    let loaded = nvs.load();
    if loaded.is_none() {
        // Not a clean slate: an empty dedup ring is *no evidence* about past
        // commands, and the caller treats it as a reason to refuse autonomous
        // actuation rather than as permission.
        log::warn!("nvs empty or unreadable; starting fresh and reporting nvs_reset");
    }
    let mut state = loaded.unwrap_or_default();
    let identity = rhizo_node_app::identity::begin_boot(&mut state, None, &mut rng);
    let interrupted = rhizo_node_app::recovery::report_interrupted(&mut state, &mut nvs);
    if let Some(result) = interrupted.as_ref() {
        log::warn!(
            "interrupted dose {} reported with delivered_ml = null",
            result.command_id
        );
    }
    if let Err(error) = nvs.store(&state) {
        log::error!("nvs commit failed at boot: {error}");
    }

    let mac = read_mac();
    let Some(device_id) = rhizo_node_app::identity::resolve(&state, mac) else {
        log::error!("no usable device identity; provision one over serial");
        halt();
    };
    log::info!(
        "identity {device_id}, boot_generation {}",
        identity.boot_generation
    );

    // ---------------------------------------------------------------- STEP 6
    // Network. Everything above this line has already happened whether or not
    // there is a router in the building.
    let sysloop = match EspSystemEventLoop::take() {
        Ok(sysloop) => sysloop,
        Err(error) => {
            log::error!("event loop unavailable: {error}");
            halt();
        }
    };

    let (Some(ssid), Some(host)) = (
        state.provisioning.wifi_ssid.clone(),
        state.provisioning.mqtt_host.clone(),
    ) else {
        // Unprovisioned. Not a fault and not a reason to retry for ever: the
        // device needs a human with a serial cable, and saying so once is more
        // use than a connection attempt every two seconds.
        log::warn!(
            "unprovisioned: run `provision wifi <ssid> <psk>`, \
             `provision mqtt <host> <user> <pass>`, then `provision commit`"
        );
        halt();
    };
    let psk = state.provisioning.wifi_psk.clone().unwrap_or_default();

    // The radio is moved out of the board before the pump is borrowed for the
    // loop, so the two do not contend for `board`.
    let modem = board.modem;
    let mut pump = board.pump;

    let mut station = match net::wifi::Station::new(modem, sysloop, partition) {
        Ok(station) => station,
        Err(error) => {
            log::error!("wifi driver unavailable: {error}");
            halt();
        }
    };
    if let Err(error) = station.configure(&ssid, &psk) {
        log::error!("wifi configuration rejected: {error}");
        halt();
    }

    let url = run::broker_url(&host);
    let mut clock = hal::clock::EdgeClock::new();
    let mut cycle = run::wake_cycle(&state, wake_reason);
    let mut ring = rhizo_node_app::telemetry::TelemetryRing::new();
    let mut attempt = 0u32;

    // ---------------------------------------------------------------- STEP 7
    // Connect, serve, reconnect. **Unlimited**, with full-jitter backoff capped
    // at 300 s: a device that gives up after N attempts is a device that needs
    // a human to power-cycle it, which is the failure mode this project exists
    // to avoid.
    loop {
        watchdog.feed();

        if !station.is_up() {
            if let Err(error) = station.connect_once() {
                let delay = station.note_failure(&mut rng);
                log::warn!("wifi attempt {attempt} failed ({error}); retrying in {delay}ms");
                attempt = attempt.saturating_add(1);
                sleep_feeding(&mut watchdog, delay);
                continue;
            }
            attempt = 0;
            log::info!("wifi up, rssi {:?}", station.rssi_dbm());
        }

        let mut context = run::Context {
            state: &mut state,
            nvs: &mut nvs,
            pump: &mut pump,
            identity,
            device_id: device_id.clone(),
            rng,
        };
        let opened = {
            let settings = run::settings(context.state, &url, &device_id);
            net::session::Session::open(&settings, identity.boot_generation)
        };
        match opened {
            Ok(mut session) => {
                run::serve(
                    &mut context,
                    &mut session,
                    &mut clock,
                    &mut cycle,
                    &mut ring,
                );
                log::warn!("session ended; reconnecting");
            }
            Err(error) => {
                let delay = crate::net::wifi::backoff_delay_ms(attempt, &mut rng);
                log::warn!("mqtt connect failed ({error}); retrying in {delay}ms");
                attempt = attempt.saturating_add(1);
                sleep_feeding(&mut watchdog, delay);
            }
        }
    }
}

/// Waits, feeding the watchdog, so a long backoff is not a watchdog reset.
fn sleep_feeding(watchdog: &mut hal::watchdog::TaskWatchdog, mut remaining_ms: u64) {
    while remaining_ms > 0 {
        let slice = remaining_ms.min(1000);
        esp_idf_hal::delay::FreeRtos::delay_ms(slice as u32);
        watchdog.feed();
        remaining_ms -= slice;
    }
}

/// Stops, leaving the pump undriven and the watchdog to reset if it must.
///
/// Deliberately not a panic: a panic unwinds or aborts through paths that have
/// nothing to do with this device's safety story, whereas a hang is a state the
/// watchdog already handles and whose result — a reset into step 1 — is the
/// safest thing that can happen.
fn halt() -> ! {
    loop {
        esp_idf_hal::delay::FreeRtos::delay_ms(1000);
    }
}

/// The station MAC, from which the default identity is derived.
fn read_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    // SAFETY: writes exactly six bytes to the buffer; no other preconditions.
    unsafe {
        esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac
}
