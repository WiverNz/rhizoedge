//! Wi-Fi, with the reconnection behaviour that matters more than anything else
//! in this firmware except the safety gate (M9-008, F-090-10).
//!
//! A device spends its life on a domestic router that reboots, changes channel,
//! and occasionally refuses DHCP for a minute. **Retry is unlimited.** A device
//! that gives up after N attempts is a device that needs a human to
//! power-cycle it, which is the failure mode this project exists to avoid.
//!
//! # Full jitter, base 2 s, cap 300 s
//!
//! Full jitter rather than a fixed schedule because a household that loses
//! power has every node retrying in step; without jitter they would all hit the
//! router in the same second, for ever.
//!
//! # Sampling continues while disconnected
//!
//! An isolated device is still a monitoring device. Whether it may *water*
//! while isolated is decided by its persisted offline policy, never by this
//! layer.

use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::{EspSubscription, EspSystemEventLoop, System};
use esp_idf_svc::netif::IpEvent;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi, WifiEvent,
};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use rhizo_node_app::ports::Rng;

/// Backoff base, in milliseconds.
pub const BACKOFF_BASE_MS: u64 = 2_000;
/// Backoff cap, in milliseconds.
pub const BACKOFF_CAP_MS: u64 = 300_000;

/// The full-jitter delay before attempt number `attempt` (zero-based).
///
/// `random_between(0, min(cap, base * 2^attempt))` — the standard full-jitter
/// form. The exponent is saturated before the shift so a long outage cannot
/// overflow it into a small delay.
#[must_use]
pub fn backoff_delay_ms(attempt: u32, rng: &mut impl Rng) -> u64 {
    let shift = attempt.min(63);
    let ceiling = BACKOFF_BASE_MS
        .saturating_mul(1u64 << shift)
        .min(BACKOFF_CAP_MS);
    let mut bytes = [0u8; 8];
    rng.fill(&mut bytes);
    u64::from_le_bytes(bytes) % (ceiling + 1)
}

/// The Wi-Fi station, with credentials from NVS.
pub struct Station<'d> {
    wifi: BlockingWifi<EspWifi<'d>>,
    attempt: u32,
    diagnostic_attempt: Arc<AtomicU32>,
    _wifi_events: EspSubscription<'static, System>,
    _ip_events: EspSubscription<'static, System>,
}

impl<'d> Station<'d> {
    /// Brings up the driver. Does not connect.
    ///
    /// # Errors
    ///
    /// If the driver or the event loop cannot be created.
    pub fn new(
        modem: Modem<'d>,
        sysloop: EspSystemEventLoop,
        partition: EspDefaultNvsPartition,
    ) -> Result<Self, esp_idf_sys::EspError> {
        trace(0, "driver_init_begin", None);
        let diagnostic_attempt = Arc::new(AtomicU32::new(0));
        let current = diagnostic_attempt.clone();
        let wifi_events = sysloop.subscribe::<WifiEvent, _>(move |event| {
            let (name, reason) = match event {
                WifiEvent::StaStarted => ("sta_start", None),
                WifiEvent::StaStopped => ("sta_stop", None),
                WifiEvent::ScanDone(_) => ("scan_done", None),
                WifiEvent::StaConnected(_) => ("sta_connected", None),
                WifiEvent::StaDisconnected(data) => ("sta_disconnected", Some(data.reason())),
                WifiEvent::StaAuthmodeChanged => ("authmode_changed", None),
                WifiEvent::StaBeaconTimeout => ("beacon_timeout", None),
                _ => return,
            };
            trace(current.load(Ordering::Relaxed), name, reason);
        })?;
        let current = diagnostic_attempt.clone();
        let ip_events = sysloop.subscribe::<IpEvent, _>(move |event| {
            if matches!(event, IpEvent::DhcpIpAssigned(_)) {
                trace(current.load(Ordering::Relaxed), "got_ip", None);
            }
        })?;
        let wifi = EspWifi::new(modem, sysloop.clone(), Some(partition))?;
        trace(0, "driver_init_end", None);
        let wifi = BlockingWifi::wrap(wifi, sysloop)?;
        Ok(Self {
            wifi,
            attempt: 0,
            diagnostic_attempt,
            _wifi_events: wifi_events,
            _ip_events: ip_events,
        })
    }

    /// Configures the station from provisioned credentials.
    ///
    /// # Errors
    ///
    /// If the configuration is rejected or the SSID does not fit.
    pub fn configure(&mut self, ssid: &str, psk: &str) -> Result<(), esp_idf_sys::EspError> {
        let auth_method = if psk.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        };
        let config = Configuration::Client(ClientConfiguration {
            ssid: ssid.try_into().unwrap_or_default(),
            password: psk.try_into().unwrap_or_default(),
            auth_method,
            ..ClientConfiguration::default()
        });
        self.wifi.set_configuration(&config)?;
        self.wifi.start()
    }

    /// One connection attempt.
    ///
    /// # Errors
    ///
    /// If association or DHCP fails. The caller backs off and tries again,
    /// without limit.
    pub fn connect_once(&mut self) -> Result<(), esp_idf_sys::EspError> {
        let attempt = self.attempt.saturating_add(1);
        self.diagnostic_attempt.store(attempt, Ordering::Relaxed);
        trace(attempt, "connect_begin", None);
        if let Err(error) = self.wifi.connect() {
            trace(attempt, "connect_error", None);
            return Err(error);
        }
        trace(attempt, "connect_return", None);
        trace(attempt, "ip_wait_begin", None);
        if let Err(error) = self.wifi.wait_netif_up() {
            trace(attempt, "ip_wait_error", None);
            return Err(error);
        }
        trace(attempt, "ip_wait_end", None);
        self.attempt = 0;
        Ok(())
    }

    /// Records a failed attempt and returns how long to wait.
    pub fn note_failure(&mut self, rng: &mut impl Rng) -> u64 {
        let delay = backoff_delay_ms(self.attempt, rng);
        self.attempt = self.attempt.saturating_add(1);
        log::info!(
            "NET_BACKOFF at_ms={} attempt={} requested_ms={delay}",
            crate::hal::clock::monotonic_ms(),
            self.attempt
        );
        delay
    }

    /// Whether the interface is up.
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.wifi.is_up().unwrap_or(false)
    }
}

/// The current RSSI, reported in status.
#[must_use]
pub fn rssi_dbm() -> Option<i16> {
    let mut rssi: i32 = 0;
    // SAFETY: writes through a pointer to a live, caller-owned `i32` and
    // has no other preconditions. A non-`ESP_OK` return leaves the local
    // untouched, and the caller discards it rather than reading a value the
    // driver never wrote.
    let code = unsafe { esp_idf_sys::esp_wifi_sta_get_rssi(&mut rssi) };
    (code == esp_idf_sys::ESP_OK).then_some(rssi as i16)
}

/// Callback receipt time, not an RF timestamp. Attempt is the active call label;
/// a delayed driver event may originate in an earlier call. Never log event Debug.
pub fn trace(attempt: u32, event: &'static str, reason: Option<u16>) {
    log::info!(
        "{}",
        super::diagnostic::Record {
            at_ms: crate::hal::clock::monotonic_ms(),
            attempt,
            event,
            reason,
        }
    );
}
