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
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

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
        let wifi = EspWifi::new(modem, sysloop.clone(), Some(partition))?;
        let wifi = BlockingWifi::wrap(wifi, sysloop)?;
        Ok(Self { wifi, attempt: 0 })
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
        self.wifi.connect()?;
        self.wifi.wait_netif_up()?;
        self.attempt = 0;
        Ok(())
    }

    /// Records a failed attempt and returns how long to wait.
    pub fn note_failure(&mut self, rng: &mut impl Rng) -> u64 {
        let delay = backoff_delay_ms(self.attempt, rng);
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Whether the interface is up.
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.wifi.is_up().unwrap_or(false)
    }

    /// The current RSSI, reported in status.
    #[must_use]
    pub fn rssi_dbm(&self) -> Option<i16> {
        // SAFETY: fills a caller-owned `i32`; returns non-zero on failure.
        let mut rssi: i32 = 0;
        let code = unsafe { esp_idf_sys::esp_wifi_sta_get_rssi(&mut rssi) };
        (code == esp_idf_sys::ESP_OK).then_some(rssi as i16)
    }
}
