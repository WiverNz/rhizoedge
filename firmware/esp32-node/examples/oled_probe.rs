//! Bench probe for the SH1106 128x64 I2C OLED, a development accessory only.
//!
//! Wiring under test is the permanent `devkitm1` I2C pair: **SDA = GPIO3,
//! SCL = GPIO2** (adjacent, J1 pins 5 and 4), VCC = 3V3, GND = GND (pin plan
//! in `src/board/devkitm1.rs`).
//! Scans the bus, initialises the SH1106 at the address it found (verified
//! 0x3C), and draws a frame and three lines of text. Every log line starts
//! with `OLED_PROBE` so a serial capture can be filtered.
//!
//! GPIO2 is a strapping pin. SCL may sit there only because the master is the
//! sole driver of SCL and the bus pull-up holds it at the level the datasheet
//! recommends at reset. When re-verifying, check the ROM's reset line: with the
//! bus pull-up present it reads `boot:0xd (SPI_FAST_FLASH_BOOT)` (bit 0 is
//! GPIO2, sampled high); a floating GPIO2 reads `boot:0xc`. Either is SPI boot.
//!
//! **Not part of the image, and not a board profile.** It does not construct
//! `board::ActiveBoard`, so it drives no pump and no rail, and it touches no
//! NVS. Restore the node image afterwards with an ordinary `espflash flash`.
//! Nothing in the image depends on the display, and it is outside every power
//! budget.

use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_idf_hal::delay::{FreeRtos, TickType};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::Hertz;
use sh1106::prelude::*;
use sh1106::Builder;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    match run() {
        Ok(addr) => log::info!("OLED_PROBE RESULT ok addr=0x{addr:02X}"),
        Err(e) => log::error!("OLED_PROBE RESULT fail: {e}"),
    }
    loop {
        FreeRtos::delay_ms(10_000);
        log::info!("OLED_PROBE idle");
    }
}

fn run() -> Result<u8, String> {
    let peripherals = Peripherals::take().map_err(|e| format!("peripherals: {e}"))?;
    let config = I2cConfig::new().baudrate(Hertz(100_000));
    let mut i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio3,
        peripherals.pins.gpio2,
        &config,
    )
    .map_err(|e| format!("i2c driver install: {e}"))?;
    log::info!("OLED_PROBE I2C0 up: SDA=GPIO3 SCL=GPIO2 100kHz internal pull-ups on");

    // An empty write is START, address+W, STOP: it asks only "does anyone ACK".
    let timeout = TickType::new_millis(20).ticks();
    let mut found = Vec::new();
    let mut nacks = 0_u32;
    for addr in 0x08_u8..=0x77 {
        match i2c.write(addr, &[], timeout) {
            Ok(()) => {
                log::info!("OLED_PROBE I2C_SCAN ack 0x{addr:02X}");
                found.push(addr);
            }
            Err(e) if e.code() == esp_idf_sys::ESP_FAIL => nacks += 1,
            Err(e) => log::warn!("OLED_PROBE I2C_SCAN 0x{addr:02X} error {e}"),
        }
    }
    let list: Vec<String> = found.iter().map(|a| format!("0x{a:02X}")).collect();
    log::info!(
        "OLED_PROBE I2C_SCAN done: {} device(s) [{}], {nacks} nack",
        found.len(),
        list.join(", ")
    );

    let addr = match found.as_slice() {
        [] => return Err("no I2C device acknowledged; check wiring and power".into()),
        [only] => *only,
        many => *many
            .iter()
            .find(|a| matches!(a, 0x3C | 0x3D))
            .ok_or("several devices and none at 0x3C/0x3D")?,
    };

    // SH1106 status read: bit 6 is ON/OFF (1 = display off), bit 7 BUSY.
    let mut status = [0_u8; 1];
    match i2c.read(addr, &mut status, timeout) {
        Ok(()) => log::info!("OLED_PROBE STATUS pre-init 0x{:02X}", status[0]),
        Err(e) => log::warn!("OLED_PROBE STATUS read failed: {e}"),
    }

    let mut display: GraphicsMode<_> = Builder::new()
        .with_size(DisplaySize::Display128x64)
        .with_i2c_addr(addr)
        .connect_i2c(i2c)
        .into();
    display.init().map_err(|e| format!("sh1106 init: {e:?}"))?;
    log::info!("OLED_PROBE SH1106 init ok addr=0x{addr:02X}");

    display.clear();
    let text = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let addr_line = format!("I2C 0x{addr:02X}");
    // The frame shows whether the 2-column SH1106 offset is right: all four
    // edges visible and none wrapped means it is.
    let drawn = Rectangle::new(Point::zero(), Size::new(128, 64))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(&mut display)
        .and_then(|()| Text::new("Rhizo Edge", Point::new(6, 16), text).draw(&mut display))
        .and_then(|_| Text::new("OLED OK", Point::new(6, 32), text).draw(&mut display))
        .and_then(|_| Text::new(&addr_line, Point::new(6, 48), text).draw(&mut display));
    if let Err(never) = drawn {
        match never {}
    }
    display
        .flush()
        .map_err(|e| format!("sh1106 flush: {e:?}"))?;
    log::info!(
        "OLED_PROBE SH1106 flush ok: frame + \"Rhizo Edge\" / \"OLED OK\" / \"{addr_line}\""
    );

    Ok(addr)
}
