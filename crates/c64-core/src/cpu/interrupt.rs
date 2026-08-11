// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - NMOS 6510 interrupt sampling and input latches
//
//   File:       interrupt.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

const RETURN_FROM_INTERRUPT_OPCODE: u8 = 0x40;
const BREAK_OPCODE: u8 = 0x00;
const NORMAL_RECOGNITION_CYCLES: u64 = 2;
const BRANCH_RECOGNITION_CYCLES: u64 = 3;
const IRQ_DEASSERTION_HOLD_CYCLES: u64 = 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
enum InterruptMaskTransition {
    #[default]
    Unchanged,
    BecameDisabled,
    BecameEnabled,
}

/// NMOS 6502/6510 instruction-boundary interrupt recognition state.
///
/// Physical IRQ/NMI line state remains the machine scheduler's responsibility.
/// This value only models the CPU-internal I-flag delay, the extra recognition
/// cycle after a taken branch, and BRK's shared interrupt-entry semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct CpuInterruptTiming {
    current_instruction_delays_interrupt: bool,
    previous_instruction_delays_interrupt: bool,
    previous_mask_transition: InterruptMaskTransition,
    previous_opcode: Option<u8>,
}

impl Default for CpuInterruptTiming {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuInterruptTiming {
    pub const fn new() -> Self {
        Self {
            current_instruction_delays_interrupt: false,
            previous_instruction_delays_interrupt: false,
            previous_mask_transition: InterruptMaskTransition::Unchanged,
            previous_opcode: None,
        }
    }

    pub fn begin_instruction(&mut self) {
        self.current_instruction_delays_interrupt = false;
    }

    pub fn delay_interrupt_for_taken_branch(&mut self) {
        self.current_instruction_delays_interrupt = true;
    }

    pub fn complete_instruction(
        &mut self,
        interrupt_masked_after: bool,
        interrupt_masked_before: bool,
        opcode: u8,
    ) {
        self.previous_mask_transition =
            if matches!(opcode, RETURN_FROM_INTERRUPT_OPCODE | BREAK_OPCODE)
                || interrupt_masked_before == interrupt_masked_after
            {
                InterruptMaskTransition::Unchanged
            } else if interrupt_masked_after {
                InterruptMaskTransition::BecameDisabled
            } else {
                InterruptMaskTransition::BecameEnabled
            };
        self.previous_instruction_delays_interrupt = self.current_instruction_delays_interrupt;
        self.previous_opcode = Some(opcode);
    }

    pub fn can_accept_maskable_interrupt(
        self,
        asserted_cycles: u64,
        interrupt_masked: bool,
    ) -> bool {
        if asserted_cycles < self.required_recognition_cycles()
            || self.previous_mask_transition == InterruptMaskTransition::BecameEnabled
        {
            return false;
        }
        !interrupt_masked
            || self.previous_mask_transition == InterruptMaskTransition::BecameDisabled
    }

    pub fn can_accept_non_maskable_interrupt(self, asserted_cycles: u64) -> bool {
        self.previous_opcode != Some(BREAK_OPCODE)
            && asserted_cycles >= self.required_recognition_cycles()
    }

    pub const fn can_take_over_interrupt_sequence_with_nmi(self, asserted_cycles: u64) -> bool {
        asserted_cycles >= NORMAL_RECOGNITION_CYCLES
    }

    pub fn complete_interrupt_entry(&mut self) {
        self.previous_mask_transition = InterruptMaskTransition::Unchanged;
        self.previous_instruction_delays_interrupt = false;
        self.previous_opcode = Some(BREAK_OPCODE);
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    const fn required_recognition_cycles(self) -> u64 {
        if self.previous_instruction_delays_interrupt {
            BRANCH_RECOGNITION_CYCLES
        } else {
            NORMAL_RECOGNITION_CYCLES
        }
    }
}

/// Low-active IRQ input latch including the NMOS pulse recognition window.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct CpuIrqLine {
    asserted: bool,
    asserted_at_cycle: Option<u64>,
    pending_until_cycle: Option<u64>,
}

impl CpuIrqLine {
    pub const fn new() -> Self {
        Self {
            asserted: false,
            asserted_at_cycle: None,
            pending_until_cycle: None,
        }
    }

    pub fn update(&mut self, asserted: bool, cycle: u64) {
        if !asserted && !self.asserted && self.pending_until_cycle.is_none() {
            return;
        }
        if asserted && !self.asserted {
            self.asserted = true;
            self.asserted_at_cycle = Some(cycle);
            self.pending_until_cycle = None;
            return;
        }

        if !asserted && self.asserted {
            self.asserted = false;
            self.pending_until_cycle = Some(cycle.saturating_add(IRQ_DEASSERTION_HOLD_CYCLES));
        }

        if !self.asserted
            && self
                .pending_until_cycle
                .is_some_and(|pending_until| cycle >= pending_until)
        {
            self.asserted_at_cycle = None;
            self.pending_until_cycle = None;
        }
    }

    pub fn is_pending(self, cycle: u64) -> bool {
        self.asserted
            || self
                .pending_until_cycle
                .is_some_and(|pending_until| cycle < pending_until)
    }

    pub fn asserted_cycles(self, cycle: u64) -> u64 {
        self.asserted_at_cycle
            .map_or(0, |asserted_at| cycle.saturating_sub(asserted_at))
    }

    pub fn complete_cpu_boundary_poll(&mut self) {
        if !self.asserted {
            self.asserted_at_cycle = None;
            self.pending_until_cycle = None;
        }
    }

    pub fn acknowledge(&mut self) {
        self.pending_until_cycle = None;
        if !self.asserted {
            self.asserted_at_cycle = None;
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Edge-sensitive NMI latch. A held physical level cannot retrigger until it
/// first becomes inactive and then asserts again.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct CpuNmiLine {
    asserted: bool,
    edge_at_cycle: Option<u64>,
    edge_pending: bool,
}

impl CpuNmiLine {
    pub const fn new() -> Self {
        Self {
            asserted: false,
            edge_at_cycle: None,
            edge_pending: false,
        }
    }

    pub fn update(&mut self, asserted: bool, cycle: u64) {
        if asserted == self.asserted {
            return;
        }
        if asserted && !self.asserted && !self.edge_pending {
            self.edge_at_cycle = Some(cycle);
            self.edge_pending = true;
        }
        self.asserted = asserted;
    }

    pub const fn is_pending(self) -> bool {
        self.edge_pending
    }

    pub fn elapsed_cycles(self, cycle: u64) -> u64 {
        self.edge_at_cycle
            .map_or(0, |edge_at| cycle.saturating_sub(edge_at))
    }

    pub fn acknowledge(&mut self) {
        self.edge_at_cycle = None;
        self.edge_pending = false;
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::{CpuInterruptTiming, CpuIrqLine, CpuNmiLine};

    const RTI: u8 = 0x40;
    const CLI: u8 = 0x58;
    const BNE: u8 = 0xd0;
    const SEI: u8 = 0x78;
    const NOP: u8 = 0xea;
    const BRK: u8 = 0x00;
    const PHA: u8 = 0x48;

    fn complete_instruction(
        timing: &mut CpuInterruptTiming,
        opcode: u8,
        masked_before: bool,
        masked_after: bool,
    ) {
        timing.begin_instruction();
        timing.complete_instruction(masked_after, masked_before, opcode);
    }

    #[test]
    fn mask_transitions_match_nmos_boundary_sampling() {
        let mut timing = CpuInterruptTiming::new();
        complete_instruction(&mut timing, NOP, false, false);
        assert!(!timing.can_accept_maskable_interrupt(1, false));
        assert!(timing.can_accept_maskable_interrupt(2, false));

        complete_instruction(&mut timing, CLI, true, false);
        assert!(!timing.can_accept_maskable_interrupt(2, false));
        complete_instruction(&mut timing, NOP, false, false);
        assert!(timing.can_accept_maskable_interrupt(2, false));

        complete_instruction(&mut timing, SEI, false, true);
        assert!(timing.can_accept_maskable_interrupt(2, true));
        complete_instruction(&mut timing, RTI, true, false);
        assert!(timing.can_accept_maskable_interrupt(2, false));
        complete_instruction(&mut timing, BRK, false, true);
        assert!(!timing.can_accept_maskable_interrupt(10, true));
    }

    #[test]
    fn taken_branch_and_brk_apply_nmi_recognition_rules() {
        let mut timing = CpuInterruptTiming::new();
        timing.begin_instruction();
        timing.delay_interrupt_for_taken_branch();
        timing.complete_instruction(false, false, BNE);
        assert!(!timing.can_accept_non_maskable_interrupt(2));
        assert!(timing.can_accept_non_maskable_interrupt(3));

        complete_instruction(&mut timing, BRK, false, true);
        assert!(!timing.can_accept_non_maskable_interrupt(10));
        timing.complete_interrupt_entry();
        assert!(!timing.can_accept_non_maskable_interrupt(10));
        complete_instruction(&mut timing, PHA, true, true);
        assert!(timing.can_accept_non_maskable_interrupt(10));
        assert!(timing.can_take_over_interrupt_sequence_with_nmi(2));
    }

    #[test]
    fn irq_latch_retains_short_pulses_for_one_boundary_poll() {
        let mut line = CpuIrqLine::new();
        line.update(true, 10);
        assert_eq!(line.asserted_cycles(12), 2);
        line.update(false, 12);
        assert!(line.is_pending(14));
        line.complete_cpu_boundary_poll();
        assert!(!line.is_pending(14));

        line.update(true, 20);
        line.acknowledge();
        assert!(line.is_pending(21));
        line.update(false, 22);
        line.update(false, 25);
        assert!(!line.is_pending(25));
    }

    #[test]
    fn nmi_latch_is_edge_sensitive() {
        let mut line = CpuNmiLine::new();
        line.update(true, 10);
        line.update(false, 11);
        assert!(line.is_pending());
        assert_eq!(line.elapsed_cycles(13), 3);
        line.update(true, 12);
        assert_eq!(line.elapsed_cycles(13), 3);
        line.acknowledge();
        line.update(true, 14);
        assert!(!line.is_pending());
        line.update(false, 15);
        line.update(true, 16);
        assert!(line.is_pending());
        assert_eq!(line.elapsed_cycles(16), 0);
    }
}
