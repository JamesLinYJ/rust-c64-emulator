// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 6510 处理器端口
//
//   文件:       processor_port.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

const PULL_UP_MASK: u8 = 0x17;
const FLOATING_PIN_MASK: u8 = 0xc0;
const DEFAULT_FLOATING_PIN_FALL_OFF_CYCLES: u32 = 350_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessorPortOutputState {
    pub direction: u8,
    pub output_latch: u8,
    pub output_pins: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessorPortInputState {
    pub mask: u8,
    pub value: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessorPort6510 {
    direction: u8,
    output: u8,
    input_pins: u8,
    floating_pin_charge: u8,
    bit6_fall_off_remaining: u32,
    bit7_fall_off_remaining: u32,
    floating_pin_fall_off_cycles: u32,
}

impl Default for ProcessorPort6510 {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessorPort6510 {
    pub const fn new() -> Self {
        Self::with_fall_off_cycles(DEFAULT_FLOATING_PIN_FALL_OFF_CYCLES)
    }

    pub const fn with_fall_off_cycles(cycles: u32) -> Self {
        Self {
            direction: 0,
            output: 0,
            input_pins: PULL_UP_MASK,
            floating_pin_charge: 0,
            bit6_fall_off_remaining: 0,
            bit7_fall_off_remaining: 0,
            floating_pin_fall_off_cycles: if cycles == 0 { 1 } else { cycles },
        }
    }

    pub const fn direction_register(&self) -> u8 {
        self.direction
    }

    pub const fn output_latch(&self) -> u8 {
        self.output
    }

    pub const fn output_pins(&self) -> u8 {
        (self.output & self.direction) | !self.direction
    }

    pub const fn data_register(&self) -> u8 {
        let input_mask = !self.direction;
        let connected_inputs = self.input_pins & input_mask;
        let floating_inputs = self.floating_pin_charge & input_mask & FLOATING_PIN_MASK;
        (self.output & self.direction) | connected_inputs | floating_inputs
    }

    pub const fn banking_configuration(&self) -> u8 {
        self.data_register() & 0x07
    }

    pub const fn output_state(&self) -> ProcessorPortOutputState {
        ProcessorPortOutputState {
            direction: self.direction,
            output_latch: self.output,
            output_pins: self.output_pins(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::with_fall_off_cycles(self.floating_pin_fall_off_cycles);
    }

    /// 写 DDR，并返回外部输出引脚是否发生变化。
    pub fn write_direction(&mut self, value: u8) -> bool {
        let previous_pins = self.output_pins();
        let switched_to_input = self.direction & !value & FLOATING_PIN_MASK;
        self.charge_floating_pins(switched_to_input, self.output);
        self.direction = value;
        self.output_pins() != previous_pins
    }

    /// 写数据锁存器，并返回外部输出引脚是否发生变化。
    pub fn write_data(&mut self, value: u8) -> bool {
        let previous_pins = self.output_pins();
        let driven_floating_pins = self.direction & FLOATING_PIN_MASK;
        self.charge_floating_pins(driven_floating_pins, value);
        self.output = value;
        self.output_pins() != previous_pins
    }

    pub fn set_input_pins(&mut self, mask: u8, value: u8) {
        let connected_mask = mask & !FLOATING_PIN_MASK;
        self.input_pins = (self.input_pins & !connected_mask) | (value & connected_mask);
    }

    pub fn tick(&mut self, cycles: u32) {
        self.bit6_fall_off_remaining = discharge_floating_pin(
            cycles,
            self.bit6_fall_off_remaining,
            0x40,
            &mut self.floating_pin_charge,
        );
        self.bit7_fall_off_remaining = discharge_floating_pin(
            cycles,
            self.bit7_fall_off_remaining,
            0x80,
            &mut self.floating_pin_charge,
        );
    }

    pub fn clock_cycle(&mut self) {
        self.tick(1);
    }

    fn charge_floating_pins(&mut self, mask: u8, value: u8) {
        if mask & 0x40 != 0 {
            self.floating_pin_charge = (self.floating_pin_charge & !0x40) | (value & 0x40);
            self.bit6_fall_off_remaining = if value & 0x40 == 0 {
                0
            } else {
                self.floating_pin_fall_off_cycles
            };
        }
        if mask & 0x80 != 0 {
            self.floating_pin_charge = (self.floating_pin_charge & !0x80) | (value & 0x80);
            self.bit7_fall_off_remaining = if value & 0x80 == 0 {
                0
            } else {
                self.floating_pin_fall_off_cycles
            };
        }
    }
}

fn discharge_floating_pin(elapsed: u32, remaining: u32, mask: u8, charge: &mut u8) -> u32 {
    let next = remaining.saturating_sub(elapsed);
    if remaining != 0 && next == 0 {
        *charge &= !mask;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::ProcessorPort6510;

    #[test]
    fn power_on_and_ddr_pin_equation_match_the_6510() {
        let mut port = ProcessorPort6510::new();
        assert_eq!(port.direction_register(), 0x00);
        assert_eq!(port.output_latch(), 0x00);
        assert_eq!(port.data_register(), 0x17);

        for direction in 0_u8..=u8::MAX {
            let output = direction.rotate_left(3) ^ 0xa5;
            let input = direction.rotate_right(2) ^ 0x5a;
            port.reset();
            port.write_direction(direction);
            port.write_data(output);
            port.set_input_pins(u8::MAX, input);
            let expected = (output & direction) | (input & !direction & 0x3f);
            assert_eq!(port.data_register(), expected, "DDR ${direction:02x}");
        }
    }

    #[test]
    fn floating_outputs_hold_charge_and_then_discharge() {
        let mut port = ProcessorPort6510::with_fall_off_cycles(5);
        port.write_direction(0xc0);
        port.write_data(0xc0);
        port.write_direction(0x00);
        port.write_data(0x00);
        assert_eq!(port.data_register() & 0xc0, 0xc0);
        port.tick(4);
        assert_eq!(port.data_register() & 0xc0, 0xc0);
        port.clock_cycle();
        assert_eq!(port.data_register() & 0xc0, 0x00);
    }

    #[test]
    fn pulled_up_inputs_drive_the_pla_lines() {
        let mut port = ProcessorPort6510::new();
        assert_eq!(port.banking_configuration(), 0x07);
        port.set_input_pins(0x07, 0x02);
        assert_eq!(port.banking_configuration(), 0x02);
    }
}
