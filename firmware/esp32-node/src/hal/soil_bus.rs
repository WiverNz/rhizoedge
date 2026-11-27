//! Bounded SEN0601 transactions over the board-owned UART and DFR0845.
use esp_idf_hal::{
    delay::{FreeRtos, TickType},
    uart::UartDriver,
};
use rhizo_node_app::{ports::SensorError, sensors::Exchange};
use std::time::{Duration, Instant};

/// DFR0845 handles direction automatically; no DE/RE GPIO is needed.
pub fn exchange(uart: UartDriver<'static>) -> Box<Exchange> {
    Box::new(move |request, response| {
        let failed = |_| SensorError::ReadFailed;
        // More than 3.5 characters at 9600/8N1, also between successive slaves.
        FreeRtos::delay_ms(5);
        uart.clear_rx().map_err(failed)?;
        if uart.write(request).map_err(failed)? != request.len() {
            return Err(SensorError::ReadFailed);
        }
        uart.wait_tx_done(TickType::new_millis(100).ticks())
            .map_err(failed)?;
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut len = 0;
        let mut expected = response.len();
        while len < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(SensorError::ReadFailed);
            }
            let count = uart
                .read(
                    &mut response[len..expected],
                    TickType::from(remaining).ticks(),
                )
                .map_err(failed)?;
            if count == 0 {
                return Err(SensorError::ReadFailed);
            }
            len += count;
            if len >= 2 && response[1] & 0x80 != 0 {
                expected = 5;
            }
        }
        // Reject trailing data rather than accepting the prefix of a bad frame.
        let mut extra = [0];
        match uart.read(&mut extra, TickType::new_millis(5).ticks()) {
            Err(error) if error.code() == esp_idf_sys::ESP_ERR_TIMEOUT => {}
            Ok(0) => {}
            _ => return Err(SensorError::ReadFailed),
        }
        Ok(len)
    })
}
