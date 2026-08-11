// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - NMOS 6502 固定单步参考验证
//
//   文件:       cpu_single_step.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use c64_core::cpu_generated::{OPCODE_OPERATION, operation};
use c64_core::{Cpu6510, Cpu6510State, CpuBus};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const FIXTURE_SHA256: &str = "5834ea0cd258fd24d338ac93c9e7c6a18c35656e67308ff89ed35a86576e0c38";

#[derive(Debug, Deserialize)]
struct ReferenceFixture {
    opcodes: BTreeMap<String, OpcodeFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpcodeFixture {
    prefix_sha256: String,
    samples: Vec<ReferenceCase>,
}

#[derive(Debug, Deserialize)]
struct ReferenceCase {
    name: String,
    initial: ReferenceState,
    #[serde(rename = "final")]
    final_state: ReferenceState,
    cycles: Vec<(u16, u8, String)>,
}

#[derive(Debug, Deserialize)]
struct ReferenceState {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    ram: Vec<(u16, u8)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AccessKind {
    Read,
    Write,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BusAccess {
    address: u16,
    value: u8,
    kind: AccessKind,
}

struct ReferenceBus {
    bytes: Box<[u8]>,
    initialized: Box<[bool]>,
    accesses: Vec<BusAccess>,
}

impl ReferenceBus {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x1_0000].into_boxed_slice(),
            initialized: vec![false; 0x1_0000].into_boxed_slice(),
            accesses: Vec::with_capacity(16),
        }
    }

    fn begin_case(&mut self, ram: &[(u16, u8)]) {
        self.initialized.fill(false);
        self.accesses.clear();
        for &(address, value) in ram {
            self.bytes[usize::from(address)] = value;
            self.initialized[usize::from(address)] = true;
        }
    }

    fn assert_final_memory(&self, ram: &[(u16, u8)], context: &str) {
        for &(address, expected) in ram {
            assert_eq!(
                self.bytes[usize::from(address)],
                expected,
                "{context}: final memory ${address:04x}"
            );
        }
    }
}

impl CpuBus for ReferenceBus {
    fn read(&mut self, address: u16) -> u8 {
        assert!(
            self.initialized[usize::from(address)],
            "read from uninitialized reference address ${address:04x}"
        );
        let value = self.bytes[usize::from(address)];
        self.accesses.push(BusAccess {
            address,
            value,
            kind: AccessKind::Read,
        });
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        self.accesses.push(BusAccess {
            address,
            value,
            kind: AccessKind::Write,
        });
        self.bytes[usize::from(address)] = value;
        self.initialized[usize::from(address)] = true;
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tools/reference/SingleStep6502Samples.json")
}

fn cpu_state(reference: &ReferenceState) -> Cpu6510State {
    Cpu6510State {
        accumulator: reference.a,
        index_x: reference.x,
        index_y: reference.y,
        program_counter: reference.pc,
        stack_pointer: reference.s,
        status: reference.p,
    }
}

fn expected_access(cycle: &(u16, u8, String)) -> BusAccess {
    BusAccess {
        address: cycle.0,
        value: cycle.1,
        kind: match cycle.2.as_str() {
            "read" => AccessKind::Read,
            "write" => AccessKind::Write,
            unsupported => panic!("unsupported reference access kind {unsupported}"),
        },
    }
}

#[test]
fn all_opcodes_match_pinned_single_step_bus_vectors() {
    let bytes = fs::read(fixture_path()).expect("read pinned SingleStepTests fixture");
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), FIXTURE_SHA256);
    let fixture: ReferenceFixture =
        serde_json::from_slice(&bytes).expect("parse pinned SingleStepTests fixture");
    let mut cpu = Cpu6510::new();
    let mut bus = ReferenceBus::new();
    let mut sample_count = 0_u32;

    for opcode in 0_u16..=0xff {
        let opcode_name = format!("{opcode:02x}");
        let opcode_fixture = fixture
            .opcodes
            .get(&opcode_name)
            .unwrap_or_else(|| panic!("fixture missing opcode ${opcode_name}"));
        assert_eq!(opcode_fixture.prefix_sha256.len(), 64);

        for (sample_index, sample) in opcode_fixture.samples.iter().enumerate() {
            let context = format!(
                "opcode ${opcode_name} sample {sample_index} ({})",
                sample.name
            );
            bus.begin_case(&sample.initial.ram);
            cpu.restore_state(cpu_state(&sample.initial));
            let jammed = OPCODE_OPERATION[usize::from(opcode)] == operation::JAM;

            for (cycle_index, expected) in sample.cycles.iter().enumerate() {
                let access_count = bus.accesses.len();
                let completed = cpu
                    .clock_cycle(&mut bus)
                    .unwrap_or_else(|error| panic!("{context}: cycle error: {error}"));
                assert_eq!(
                    bus.accesses.len(),
                    access_count + 1,
                    "{context}: cycle {} emitted exactly one bus access",
                    cycle_index + 1
                );
                assert_eq!(
                    bus.accesses[cycle_index],
                    expected_access(expected),
                    "{context}: bus cycle {}",
                    cycle_index + 1
                );

                if jammed {
                    assert!(!completed, "{context}: JAM exposed an instruction boundary");
                    if cycle_index >= 1 {
                        assert!(cpu.is_jammed(), "{context}: JAM did not latch");
                    }
                } else {
                    assert_eq!(
                        completed,
                        cycle_index + 1 == sample.cycles.len(),
                        "{context}: boundary on cycle {}",
                        cycle_index + 1
                    );
                }
            }

            assert_eq!(
                cpu.state(),
                cpu_state(&sample.final_state),
                "{context}: state"
            );
            bus.assert_final_memory(&sample.final_state.ram, &context);
            sample_count += 1;
        }
    }

    assert_eq!(sample_count, 4_096);
}
