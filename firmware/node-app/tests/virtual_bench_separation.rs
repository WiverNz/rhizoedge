//! Structural contracts for a compile-time-only real-board bench image.
#![allow(clippy::expect_used)]
use std::path::Path;
fn source(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../esp32-node")
            .join(name),
    )
    .expect("firmware source")
}
#[test]
fn actuator_selection_is_compile_time_and_default_remains_physical() {
    let manifest = source("Cargo.toml");
    assert!(manifest.contains("default = [\"board-devkitm1\"]"));
    let selection = source("src/board/mod.rs");
    assert!(selection.contains("#[cfg(all(feature = \"board-devkitm1\", not(feature = \"bench-virtual-actuator\")))]\nmod devkitm1;"));
    assert!(selection.contains("#[cfg(all(feature = \"board-devkitm1\", feature = \"bench-virtual-actuator\"))]\n#[path = \"virtual_devkitm1.rs\"]\nmod devkitm1;"));
    assert!(source("src/board/devkitm1.rs").contains("type Pump = GpioPump<'static>;"));
    let virtual_board = source("src/board/virtual_devkitm1.rs");
    assert!(virtual_board.contains("type Pump = VirtualPump;"));
    for forbidden in ["pins.", "PinDriver", "GpioPump", "GpioRail"] {
        assert!(!virtual_board.contains(forbidden));
        assert!(!source("src/bench.rs").contains(forbidden));
    }
    assert!(
        source("src/hal/mod.rs")
            .contains("#[cfg(not(feature = \"bench-virtual-actuator\"))]\npub mod pump;")
    );
}
#[test]
fn safe_fixtures_are_compile_time_only_and_production_remains_unknown() {
    let run = source("src/run.rs");
    for field in ["leak: LeakState::Unknown,", "tank_percent: None,"] {
        assert!(run.contains(&format!(
            "#[cfg(not(feature = \"bench-virtual-actuator\"))]\n        {field}"
        )));
    }
    for field in ["leak: LeakState::Clear,", "tank_percent: Some(75.0),"] {
        assert!(run.contains(&format!(
            "#[cfg(feature = \"bench-virtual-actuator\")]\n        {field}"
        )));
    }
    assert!(run.contains("pump_enabled: config.is_some_and(|c| c.pump.enabled)"));
}
#[test]
fn command_lifecycle_and_dedup_are_shared_without_bench_branches() {
    let run = source("src/run.rs");
    assert_eq!(run.matches("let handled = handle_water(").count(), 1);
    let command =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/command.rs"))
            .expect("command source");
    assert!(!command.contains("bench-virtual-actuator"));
    assert!(command.contains("match validate_water_command(command, &guard)"));
    assert!(command.contains("Authorisation::AlreadyExecuted(previous)"));
}

#[test]
fn flash_layout_has_capacity_for_both_maximum_nvs_slots_and_gc() {
    assert!(source("espflash.toml").contains("partition_table_offset = 32768"));
    let csv = source("partitions.csv");
    let entries: Vec<_> = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split(',').map(str::trim).collect::<Vec<_>>())
        .collect();
    let hex = |s: &str| {
        usize::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex partition field")
    };
    let nvs = entries
        .iter()
        .find(|e| e[0] == "nvs")
        .expect("NVS partition");
    let capacity = hex(nvs[4]);
    assert!(
        capacity >= 3 * 32 * 1024 + 4 * 4096,
        "two 32 KiB slots, replacement blob and GC metadata must fit"
    );
    for (i, e) in entries.iter().enumerate() {
        let (start, end) = (hex(e[3]), hex(e[3]) + hex(e[4]));
        assert!(start >= 0x9000 && end <= 4 * 1024 * 1024);
        for other in entries.iter().skip(i + 1) {
            let (a, b) = (hex(other[3]), hex(other[3]) + hex(other[4]));
            assert!(end <= a || b <= start, "partitions overlap");
        }
    }
    assert!(source("espflash.toml").contains("partition_table = \"partitions.csv\""));
}
