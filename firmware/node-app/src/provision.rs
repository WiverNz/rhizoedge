//! Serial provisioning (M9-006, ADR-012).
//!
//! One firmware image serves every device, and no binary contains a secret.
//!
//! Two properties are security properties rather than conveniences:
//!
//! * **`show` redacts.** The whole point of serial provisioning is that
//!   credentials never appear in a binary or a log, and a console that prints
//!   them back undoes that.
//! * **The console is closed once the network is up.** A serial console
//!   reachable at runtime is a credential-disclosure path on a device somebody
//!   might place in a shared space, so provisioning is available only in the
//!   pre-network window or behind an explicit unlock.

use crate::persist::PersistedState;

/// What the console will accept right now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConsoleState {
    /// Before network initialisation: provisioning commands are accepted.
    #[default]
    Open,
    /// After network initialisation: refused unless explicitly unlocked.
    Closed,
    /// Explicitly unlocked at runtime by an operator with serial access.
    Unlocked,
}

impl ConsoleState {
    /// Whether a provisioning command may run.
    #[must_use]
    pub const fn accepts_commands(self) -> bool {
        matches!(self, Self::Open | Self::Unlocked)
    }
}

/// The outcome of one console line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleResponse {
    /// A value was staged; `commit` will persist it.
    Staged(String),
    /// The staged values were persisted.
    Committed,
    /// A redacted view of the staged and stored settings.
    Shown(String),
    /// The line was not understood.
    Unknown(String),
    /// The console is closed.
    Refused,
}

/// The provisioning console.
#[derive(Clone, Debug, Default)]
pub struct Console {
    state: ConsoleState,
    staged: crate::persist::Provisioning,
}

impl Console {
    /// Handles a line, committing provisioning changes before reporting success.
    ///
    /// # Errors
    /// Returns a storage error and restores the live credentials on failure.
    pub fn handle_durable(
        &mut self,
        line: &str,
        state: &mut PersistedState,
        store: &mut impl crate::ports::NvsStore,
    ) -> Result<ConsoleResponse, crate::ports::NvsError> {
        let previous = state.provisioning.clone();
        let response = self.handle(line, state);
        if response != ConsoleResponse::Committed {
            return Ok(response);
        }
        if let Err(error) = store.store(state) {
            state.provisioning = previous;
            return Err(error);
        }
        Ok(response)
    }
    /// A console open for provisioning, as at the top of `main`.
    #[must_use]
    pub fn open(existing: &PersistedState) -> Self {
        Self {
            state: ConsoleState::Open,
            staged: existing.provisioning.clone(),
        }
    }

    /// Closes the console, as the network comes up.
    pub fn close(&mut self) {
        self.state = ConsoleState::Closed;
    }

    /// Unlocks a closed console.
    pub fn unlock(&mut self) {
        self.state = ConsoleState::Unlocked;
    }

    /// The current console state.
    #[must_use]
    pub const fn state(&self) -> ConsoleState {
        self.state
    }

    /// Handles one console line.
    pub fn handle(&mut self, line: &str, state: &mut PersistedState) -> ConsoleResponse {
        let trimmed = line.trim();
        if trimmed == "provision unlock" {
            self.unlock();
            return ConsoleResponse::Staged("unlocked".into());
        }
        if !self.state.accepts_commands() {
            return ConsoleResponse::Refused;
        }
        let mut parts = trimmed.split_whitespace();
        if parts.next() != Some("provision") {
            return ConsoleResponse::Unknown(trimmed.to_owned());
        }
        match parts.next() {
            Some("wifi") => match (parts.next(), parts.next()) {
                (Some(ssid), Some(psk)) => {
                    self.staged.wifi_ssid = Some(ssid.to_owned());
                    self.staged.wifi_psk = Some(psk.to_owned());
                    ConsoleResponse::Staged("wifi".into())
                }
                _ => ConsoleResponse::Unknown(trimmed.to_owned()),
            },
            Some("mqtt") => match (parts.next(), parts.next(), parts.next()) {
                (Some(host), Some(user), Some(pass)) => {
                    self.staged.mqtt_host = Some(host.to_owned());
                    self.staged.mqtt_user = Some(user.to_owned());
                    self.staged.mqtt_pass = Some(pass.to_owned());
                    ConsoleResponse::Staged("mqtt".into())
                }
                _ => ConsoleResponse::Unknown(trimmed.to_owned()),
            },
            Some("device-id") => match parts.next() {
                Some(id) => {
                    self.staged.device_id = Some(id.to_owned());
                    ConsoleResponse::Staged("device-id".into())
                }
                None => ConsoleResponse::Unknown(trimmed.to_owned()),
            },
            Some("show") => ConsoleResponse::Shown(self.redacted()),
            Some("commit") => {
                state.provisioning = self.staged.clone();
                ConsoleResponse::Committed
            }
            _ => ConsoleResponse::Unknown(trimmed.to_owned()),
        }
    }

    /// A redacted view. Secrets are never echoed, only their presence.
    #[must_use]
    pub fn redacted(&self) -> String {
        fn shown(value: Option<&String>) -> &str {
            value.map_or("<unset>", |value| value.as_str())
        }
        fn hidden(value: Option<&String>) -> &'static str {
            if value.is_some() { "<set>" } else { "<unset>" }
        }
        format!(
            "device-id {}\nwifi_ssid {}\nwifi_psk {}\nmqtt_host {}\nmqtt_user {}\nmqtt_pass {}",
            shown(self.staged.device_id.as_ref()),
            shown(self.staged.wifi_ssid.as_ref()),
            hidden(self.staged.wifi_psk.as_ref()),
            shown(self.staged.mqtt_host.as_ref()),
            shown(self.staged.mqtt_user.as_ref()),
            hidden(self.staged.mqtt_pass.as_ref()),
        )
    }
}

/// Bounded serial line framing. An oversized line is discarded through its
/// terminator so its suffix can never become a second provisioning command.
#[derive(Default)]
pub struct LineBuffer {
    bytes: Vec<u8>,
    overflow: bool,
}

impl LineBuffer {
    /// Feeds one byte. Invalid UTF-8 and oversized lines return an empty line;
    /// no rejected input is echoed (it may contain credentials).
    pub fn push(&mut self, byte: u8) -> Option<String> {
        if byte == b'\n' || byte == b'\r' {
            let bytes = core::mem::take(&mut self.bytes);
            let overflow = core::mem::replace(&mut self.overflow, false);
            return Some(if overflow {
                String::new()
            } else {
                String::from_utf8(bytes).unwrap_or_default()
            });
        }
        if self.bytes.len() >= 512 {
            self.overflow = true;
        } else if !self.overflow {
            self.bytes.push(byte);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_serial_line_cannot_execute_its_suffix() {
        let mut buffer = LineBuffer::default();
        for byte in [b'x'; 513]
            .into_iter()
            .chain(b"provision commit".iter().copied())
        {
            assert!(buffer.push(byte).is_none());
        }
        assert_eq!(buffer.push(b'\n'), Some(String::new()));
        for byte in b"provision show" {
            buffer.push(*byte);
        }
        assert_eq!(buffer.push(b'\r').as_deref(), Some("provision show"));
    }

    #[test]
    fn a_failed_durable_commit_keeps_the_previous_credentials() {
        use crate::ports::NvsStore;
        let mut state = PersistedState::default();
        state.provisioning.device_id = Some("original-node".into());
        let mut store = crate::fakes::FakeNvs::default();
        store.store(&state).expect("initial write");
        let mut console = Console::open(&state);
        console.handle("provision device-id replacement-node", &mut state);
        store.fail_writes(true);
        assert!(
            console
                .handle_durable("provision commit", &mut state, &mut store)
                .is_err()
        );
        assert_eq!(
            state.provisioning.device_id.as_deref(),
            Some("original-node")
        );
        assert_eq!(
            store.load().expect("old state").provisioning,
            state.provisioning
        );
    }

    #[test]
    fn provisioning_commands_stage_and_commit_to_storage() {
        let mut state = PersistedState::default();
        let mut console = Console::open(&state);
        console.handle("provision wifi homenet hunter2", &mut state);
        console.handle("provision mqtt broker.local node secret", &mut state);
        console.handle("provision device-id balcony-basil", &mut state);
        assert_eq!(
            state.provisioning.wifi_ssid, None,
            "nothing is persisted before commit"
        );
        assert_eq!(
            console.handle("provision commit", &mut state),
            ConsoleResponse::Committed
        );
        assert_eq!(state.provisioning.wifi_ssid.as_deref(), Some("homenet"));
        assert_eq!(state.provisioning.mqtt_pass.as_deref(), Some("secret"));
        assert_eq!(
            state.provisioning.device_id.as_deref(),
            Some("balcony-basil")
        );
    }

    #[test]
    fn show_redacts_every_secret() {
        let mut state = PersistedState::default();
        let mut console = Console::open(&state);
        console.handle("provision wifi homenet hunter2", &mut state);
        console.handle("provision mqtt broker.local node s3cr3t", &mut state);
        let ConsoleResponse::Shown(view) = console.handle("provision show", &mut state) else {
            panic!("show returns a view");
        };
        assert!(view.contains("homenet"), "non-secret fields are shown");
        assert!(!view.contains("hunter2"), "the PSK is never echoed");
        assert!(
            !view.contains("s3cr3t"),
            "the broker password is never echoed"
        );
        assert!(view.contains("wifi_psk <set>"));
    }

    #[test]
    fn a_closed_console_refuses_until_it_is_explicitly_unlocked() {
        let mut state = PersistedState::default();
        let mut console = Console::open(&state);
        console.close();
        assert_eq!(
            console.handle("provision wifi other pass", &mut state),
            ConsoleResponse::Refused
        );
        console.handle("provision unlock", &mut state);
        assert_eq!(console.state(), ConsoleState::Unlocked);
        assert!(matches!(
            console.handle("provision wifi other pass", &mut state),
            ConsoleResponse::Staged(_)
        ));
    }

    #[test]
    fn one_image_serves_several_devices() {
        let mut first = PersistedState::default();
        let mut second = PersistedState::default();
        let mut console = Console::open(&first);
        console.handle("provision device-id node-alpha", &mut first);
        console.handle("provision commit", &mut first);

        let mut other = Console::open(&second);
        other.handle("provision device-id node-beta", &mut second);
        other.handle("provision commit", &mut second);

        assert_eq!(first.provisioning.device_id.as_deref(), Some("node-alpha"));
        assert_eq!(second.provisioning.device_id.as_deref(), Some("node-beta"));
    }

    #[test]
    fn a_malformed_line_is_reported_rather_than_partially_applied() {
        let mut state = PersistedState::default();
        let mut console = Console::open(&state);
        assert!(matches!(
            console.handle("provision wifi only-ssid", &mut state),
            ConsoleResponse::Unknown(_)
        ));
        console.handle("provision commit", &mut state);
        assert_eq!(state.provisioning.wifi_ssid, None);
    }
}
