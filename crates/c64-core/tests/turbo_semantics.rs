// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Turbo slot and strict CPU differential tests
//
//   File:       tests/turbo_semantics.rs
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::{C64Core, MemoryWriteSource};

const PROGRAM_ADDRESS: u16 = 0x2000;
const SOURCE_ADDRESS: u16 = 0x4000;
const DESTINATION_ADDRESS: u16 = 0x4100;
const CPU_SLOTS: u64 = 4_096;
const TURBO_TIERS: [u8; 10] = [2, 4, 8, 12, 16, 20, 24, 32, 48, 64];

#[test]
fn every_public_turbo_tier_matches_strict_for_pure_ram_cpu_slots() {
    let seed = pure_ram_machine();
    let mut strict = seed.clone();
    let strict_transactions_before = strict.diagnostics().cpu_bus_transactions;
    let strict_elapsed = strict.run_cpu_slots(CPU_SLOTS).unwrap();
    let strict_transactions =
        strict.diagnostics().cpu_bus_transactions - strict_transactions_before;

    assert_eq!(strict_elapsed, CPU_SLOTS);
    assert_eq!(strict_transactions, CPU_SLOTS);
    assert_eq!(strict.diagnostics().held_cpu_read_system_cycles, 0);

    for tier in TURBO_TIERS {
        let mut turbo = seed.clone();
        turbo.request_manual_turbo(tier).unwrap();
        let transactions_before = turbo.diagnostics().cpu_bus_transactions;
        let elapsed = turbo.run_cpu_slots(CPU_SLOTS).unwrap();

        assert_eq!(
            turbo.cpu_state(),
            strict.cpu_state(),
            "CPU state diverged at {tier} slots per system cycle",
        );
        assert_eq!(
            &turbo.memory().base_ram()
                [usize::from(DESTINATION_ADDRESS)..usize::from(DESTINATION_ADDRESS) + 0x100],
            &strict.memory().base_ram()
                [usize::from(DESTINATION_ADDRESS)..usize::from(DESTINATION_ADDRESS) + 0x100],
            "RAM writes diverged at {tier} slots per system cycle",
        );
        assert_eq!(
            turbo.diagnostics().cpu_bus_transactions - transactions_before,
            strict_transactions,
        );
        assert_eq!(turbo.diagnostics().held_cpu_read_system_cycles, 0);
        assert_eq!(elapsed, CPU_SLOTS / u64::from(tier));
        assert_eq!(
            turbo.timestamp().slot,
            u8::try_from(CPU_SLOTS % u64::from(tier)).unwrap(),
        );
    }
}

fn pure_ram_machine() -> C64Core {
    let mut core = C64Core::default();
    for index in 0_u16..=0xff {
        core.write_base_ram(
            SOURCE_ADDRESS + index,
            index.to_le_bytes()[0].rotate_left(3) ^ 0xa5,
            MemoryWriteSource::HostLoader,
        );
    }

    // LDX #$00
    // loop: LDA $4000,X; EOR #$a5; ADC #$13; STA $4100,X; INX; JMP loop
    for (offset, value) in [
        0xa2, 0x00, 0xbd, 0x00, 0x40, 0x49, 0xa5, 0x69, 0x13, 0x9d, 0x00, 0x41, 0xe8, 0x4c, 0x02,
        0x20,
    ]
    .into_iter()
    .enumerate()
    {
        core.write_base_ram(
            PROGRAM_ADDRESS + u16::try_from(offset).unwrap(),
            value,
            MemoryWriteSource::HostLoader,
        );
    }
    assert!(core.set_cpu_program_counter(PROGRAM_ADDRESS));
    core
}
