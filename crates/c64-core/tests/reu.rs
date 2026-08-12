// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 17xx REU root-cause and integration tests
//
//   File:       tests/reu.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use c64_core::devices::reu::{RamExpansionUnit, ReuSize};
use c64_core::{C64Core, CoreConfig, MemoryWriteSource};

const PROGRAM_ADDRESS: u16 = 0x1000;

#[test]
fn reset_registers_mirror_and_clear_status_side_effects() {
    let mut reu = RamExpansionUnit::new(ReuSize::Kib512);

    assert_eq!(reu.peek_register(0xdf00), 0x10);
    assert_eq!(reu.peek_register(0xdf01), 0x10);
    assert_eq!(reu.peek_register(0xdf06), 0xf8);
    assert_eq!(reu.peek_register(0xdf07), 0xff);
    assert_eq!(reu.peek_register(0xdf08), 0xff);
    assert_eq!(reu.peek_register(0xdf09), 0x1f);
    assert_eq!(reu.peek_register(0xdf0a), 0x3f);
    assert_eq!(reu.peek_register(0xdf0b), 0xff);
    assert_eq!(reu.peek_register(0xdf20), 0x10);

    reu.write_register(0xdf06, 0xff);
    reu.write_register(0xdf09, 0x00);
    reu.write_register(0xdf0a, 0x00);
    assert_eq!(reu.peek_register(0xdf06), 0xff);
    assert_eq!(reu.peek_register(0xdf09), 0x1f);
    assert_eq!(reu.peek_register(0xdf0a), 0x3f);
}

#[test]
fn command_supports_immediate_and_ff00_delayed_dma_without_false_triggers() {
    let mut reu = RamExpansionUnit::new(ReuSize::Kib512);

    reu.write_register(0xdf01, 0x80);
    assert!(reu.ff00_trigger_armed());
    assert!(!reu.dma_active());
    reu.observe_cpu_write(0xfeff);
    assert!(!reu.dma_active());
    reu.observe_cpu_write(0xff00);
    assert!(reu.dma_active());

    reu.reset();
    reu.write_register(0xdf01, 0x90);
    assert!(!reu.ff00_trigger_armed());
    assert!(reu.dma_active());
}

#[test]
fn core_executes_copy_fetch_swap_and_verify_through_the_c64_bus() {
    let mut core = core_with_reu();
    for (offset, value) in [0x11, 0x22, 0x33, 0x44].into_iter().enumerate() {
        core.write_base_ram(
            0x2000 + u16::try_from(offset).unwrap(),
            value,
            MemoryWriteSource::HostLoader,
        );
    }

    run_dma_program(&mut core, 0x2000, 0x01_2345, 4, 0x90, 0x00, 0x00, false);
    assert_eq!(
        &core.reu().unwrap().ram()[0x1_2345..0x1_2349],
        &[0x11, 0x22, 0x33, 0x44]
    );
    assert_eq!(core.reu().unwrap().peek_register(0xdf02), 0x04);
    assert_eq!(core.reu().unwrap().peek_register(0xdf03), 0x20);
    assert_eq!(core.reu().unwrap().peek_register(0xdf04), 0x49);
    assert_eq!(core.reu().unwrap().peek_register(0xdf05), 0x23);
    assert_eq!(core.reu().unwrap().peek_register(0xdf06), 0xf9);
    assert_eq!(core.reu().unwrap().peek_register(0xdf07), 0x01);
    assert_eq!(core.reu().unwrap().peek_register(0xdf08), 0x00);

    core.reu_mut().unwrap().ram_mut()[0x1_2345..0x1_2349]
        .copy_from_slice(&[0x51, 0x52, 0x53, 0x54]);
    let generation_before = core.memory().memory_generation();
    run_dma_program(&mut core, 0xa000, 0x01_2345, 4, 0x91, 0x00, 0x00, false);
    assert_eq!(
        [
            core.read_base_ram(0xa000),
            core.read_base_ram(0xa001),
            core.read_base_ram(0xa002),
            core.read_base_ram(0xa003),
        ],
        [0x51, 0x52, 0x53, 0x54]
    );
    assert!(core.memory().memory_generation() >= generation_before + 4);
    assert_eq!(core.memory().code_generation(0xa0), 4);

    core.write_base_ram(0x2100, 0xa1, MemoryWriteSource::HostLoader);
    core.write_base_ram(0x2101, 0xa2, MemoryWriteSource::HostLoader);
    core.reu_mut().unwrap().ram_mut()[0x300..0x302].copy_from_slice(&[0xb1, 0xb2]);
    run_dma_program(&mut core, 0x2100, 0x300, 2, 0x92, 0x00, 0x00, false);
    assert_eq!(
        [core.read_base_ram(0x2100), core.read_base_ram(0x2101)],
        [0xb1, 0xb2]
    );
    assert_eq!(&core.reu().unwrap().ram()[0x300..0x302], &[0xa1, 0xa2]);

    let _ = core.reu_mut().unwrap().read_register(0xdf00);
    run_dma_program(&mut core, 0x2100, 0x300, 2, 0x93, 0x00, 0x00, false);
    assert_eq!(core.reu().unwrap().peek_register(0xdf00) & 0x60, 0x20);
    assert_eq!(core.reu().unwrap().peek_register(0xdf07), 0x01);
}

#[test]
fn autoload_irq_zero_page_and_512k_wrap_follow_rec_rules() {
    let mut core = core_with_reu();
    core.write_base_ram(0x0000, 0x6a, MemoryWriteSource::HostLoader);
    core.write_base_ram(0x0001, 0xb5, MemoryWriteSource::HostLoader);

    run_dma_program(&mut core, 0x0000, 0x7_ffff, 2, 0xb0, 0xc0, 0x00, false);
    assert_eq!(&core.reu().unwrap().ram()[0x7_ffff..], &[0x6a]);
    assert_eq!(core.reu().unwrap().ram()[0], 0xb5);
    assert_eq!(core.reu().unwrap().peek_register(0xdf02), 0x00);
    assert_eq!(core.reu().unwrap().peek_register(0xdf03), 0x00);
    assert_eq!(core.reu().unwrap().peek_register(0xdf04), 0xff);
    assert_eq!(core.reu().unwrap().peek_register(0xdf05), 0xff);
    assert_eq!(core.reu().unwrap().peek_register(0xdf06), 0xff);
    assert_eq!(core.reu().unwrap().peek_register(0xdf07), 0x02);
    assert_eq!(core.reu().unwrap().peek_register(0xdf08), 0x00);
    assert_eq!(core.reu().unwrap().peek_register(0xdf00) & 0xc0, 0xc0);
    assert!(core.devices().irq_asserted());
    assert_eq!(core.reu_mut().unwrap().read_register(0xdf00), 0xd0);
    assert_eq!(core.reu().unwrap().peek_register(0xdf00), 0x10);
    assert!(!core.devices().irq_asserted());
}

#[test]
fn delayed_dma_starts_only_after_the_cpu_reaches_ff00() {
    let mut core = core_with_reu();
    core.write_base_ram(0x2200, 0x7c, MemoryWriteSource::HostLoader);

    run_dma_program(&mut core, 0x2200, 0x400, 1, 0x80, 0x00, 0x00, true);

    assert_eq!(core.reu().unwrap().ram()[0x400], 0x7c);
    assert!(!core.reu().unwrap().ff00_trigger_armed());
}

#[test]
fn vic_retains_bus_priority_while_reu_dma_owns_cpu_cycles() {
    let mut core = core_with_reu();
    run_program_to_halt(&mut core, &[0xa9, 0x10, 0x8d, 0x11, 0xd0]);

    let diagnostics_before = core.diagnostics();
    run_dma_program(&mut core, 0x3000, 0, 4_096, 0x90, 0x00, 0x00, false);
    let diagnostics = core.diagnostics();
    let dma_bus_cycles = diagnostics.reu_dma_bus_cycles - diagnostics_before.reu_dma_bus_cycles;
    let vic_stalls =
        diagnostics.reu_dma_vic_stall_cycles - diagnostics_before.reu_dma_vic_stall_cycles;
    let dma_system_cycles =
        diagnostics.reu_dma_system_cycles - diagnostics_before.reu_dma_system_cycles;

    assert_eq!(dma_bus_cycles, 4_096);
    assert!(vic_stalls >= 40);
    assert_eq!(dma_system_cycles, dma_bus_cycles + vic_stalls);
}

fn core_with_reu() -> C64Core {
    let mut core = C64Core::new(CoreConfig::default());
    core.attach_reu(ReuSize::Kib512).unwrap();
    core
}

#[allow(clippy::too_many_arguments)]
fn run_dma_program(
    core: &mut C64Core,
    c64_address: u16,
    reu_address: u32,
    length: u16,
    command: u8,
    interrupt_mask: u8,
    address_control: u8,
    trigger_ff00: bool,
) {
    let mut program = Vec::new();
    emit_register_write(&mut program, 0xdf02, c64_address.to_le_bytes()[0]);
    emit_register_write(&mut program, 0xdf03, c64_address.to_le_bytes()[1]);
    emit_register_write(&mut program, 0xdf04, reu_address.to_le_bytes()[0]);
    emit_register_write(&mut program, 0xdf05, reu_address.to_le_bytes()[1]);
    emit_register_write(&mut program, 0xdf06, reu_address.to_le_bytes()[2]);
    emit_register_write(&mut program, 0xdf07, length.to_le_bytes()[0]);
    emit_register_write(&mut program, 0xdf08, length.to_le_bytes()[1]);
    emit_register_write(&mut program, 0xdf09, interrupt_mask);
    emit_register_write(&mut program, 0xdf0a, address_control);
    emit_register_write(&mut program, 0xdf01, command);
    if trigger_ff00 {
        emit_register_write(&mut program, 0xff00, 0x5a);
    }
    run_program_to_halt(core, &program);
}

fn run_program_to_halt(core: &mut C64Core, program: &[u8]) {
    let halt = PROGRAM_ADDRESS + u16::try_from(program.len()).unwrap();
    let mut program = program.to_vec();
    program.extend_from_slice(&[0x4c, halt.to_le_bytes()[0], halt.to_le_bytes()[1]]);

    for (offset, value) in (0_u16..).zip(program) {
        core.write_base_ram(
            PROGRAM_ADDRESS + offset,
            value,
            MemoryWriteSource::HostLoader,
        );
    }
    while !core.cpu_is_at_instruction_boundary() {
        core.run_cpu_slots(1).unwrap();
    }
    assert!(core.set_cpu_program_counter(PROGRAM_ADDRESS));
    for _ in 0..512 {
        core.run_cpu_slots(1).unwrap();
        if core.cpu_is_at_instruction_boundary() && core.cpu_state().program_counter == halt {
            core.run_cpu_slots(1).unwrap();
            return;
        }
    }
    panic!("REU setup program did not reach its halt loop");
}

fn emit_register_write(program: &mut Vec<u8>, address: u16, value: u8) {
    let [low, high] = address.to_le_bytes();
    program.extend_from_slice(&[0xa9, value, 0x8d, low, high]);
}
