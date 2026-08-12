// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 键盘、控制端口与 RESTORE 宿主输入
//
//   文件:       input.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const C64_KEYBOARD_MATRIX_SIDE: usize = 8;
pub const C64_CONTROL_PORT_DIGITAL_MASK: u8 = 0x1f;
pub const RESTORE_NMI_PULSE_CYCLES: u8 = 29;
const SHIFT_LOCK_ROW: usize = 7;
const SHIFT_LOCK_COLUMN: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C64HostInputPortState {
    pub data_direction: u8,
    pub external_input_pins: u8,
    pub output_pins: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C64HostInputPortValues {
    pub port_a: u8,
    pub port_b: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C64HostInputError {
    KeyboardMatrixLength { actual: usize },
    JoystickMask { port: u8, value: u8 },
}

impl fmt::Display for C64HostInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyboardMatrixLength { actual } => write!(
                formatter,
                "keyboard matrix must contain exactly {C64_KEYBOARD_MATRIX_SIDE} columns; received {actual}"
            ),
            Self::JoystickMask { port, value } => write!(
                formatter,
                "joystick port {port} lines must fit the five-bit digital mask; received {value:#04x}"
            ),
        }
    }
}

impl std::error::Error for C64HostInputError {}

/// 浏览器或原生前端提供的外部开关状态。键盘矩阵和操纵杆属于机器外部，
/// RESTORE 单稳态脉冲则在这里按主时钟推进。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C64HostInput {
    pressed_rows_by_column: [u8; C64_KEYBOARD_MATRIX_SIDE],
    joystick_port_1_grounded: u8,
    joystick_port_2_grounded: u8,
    shift_lock_pressed: bool,
    restore_key_pressed: bool,
    restore_pulse_cycles_remaining: u8,
}

impl Default for C64HostInput {
    fn default() -> Self {
        Self::new()
    }
}

impl C64HostInput {
    pub const fn new() -> Self {
        Self {
            pressed_rows_by_column: [0; C64_KEYBOARD_MATRIX_SIDE],
            joystick_port_1_grounded: 0,
            joystick_port_2_grounded: 0,
            shift_lock_pressed: false,
            restore_key_pressed: false,
            restore_pulse_cycles_remaining: 0,
        }
    }

    pub const fn nmi_asserted(&self) -> bool {
        self.restore_pulse_cycles_remaining > 0
    }

    pub const fn pressed_rows_by_column(&self) -> &[u8; C64_KEYBOARD_MATRIX_SIDE] {
        &self.pressed_rows_by_column
    }

    pub const fn joystick_port_1_grounded(&self) -> u8 {
        self.joystick_port_1_grounded
    }

    pub const fn joystick_port_2_grounded(&self) -> u8 {
        self.joystick_port_2_grounded
    }

    pub const fn shift_lock_pressed(&self) -> bool {
        self.shift_lock_pressed
    }

    pub(crate) fn keyboard_matrix_is_open(&self) -> bool {
        self.pressed_rows_by_column == [0; C64_KEYBOARD_MATRIX_SIDE] && !self.shift_lock_pressed
    }

    /// 原子替换一份宿主输入快照。
    ///
    /// # Errors
    ///
    /// 键盘列数不是八列，或任一操纵杆包含未接线的高位时拒绝更新。
    pub fn set_state(
        &mut self,
        pressed_rows_by_column: &[u8],
        shift_lock_pressed: bool,
        joystick_port_1_grounded: u8,
        joystick_port_2_grounded: u8,
        restore_key_pressed: bool,
    ) -> Result<(), C64HostInputError> {
        let rows: [u8; C64_KEYBOARD_MATRIX_SIDE] =
            pressed_rows_by_column
                .try_into()
                .map_err(|_: std::array::TryFromSliceError| {
                    C64HostInputError::KeyboardMatrixLength {
                        actual: pressed_rows_by_column.len(),
                    }
                })?;
        Self::validate_joystick_mask(1, joystick_port_1_grounded)?;
        Self::validate_joystick_mask(2, joystick_port_2_grounded)?;

        self.pressed_rows_by_column = rows;
        self.shift_lock_pressed = shift_lock_pressed;
        self.joystick_port_1_grounded = joystick_port_1_grounded;
        self.joystick_port_2_grounded = joystick_port_2_grounded;
        self.set_restore_key_pressed(restore_key_pressed);
        Ok(())
    }

    pub fn resolve_port_inputs(
        &self,
        port_a: C64HostInputPortState,
        port_b: C64HostInputPortState,
    ) -> C64HostInputPortValues {
        let mut resolved_port_a = port_a.output_pins & port_a.external_input_pins;
        let mut resolved_port_b = port_b.output_pins & port_b.external_input_pins;
        if self.keyboard_matrix_is_open() {
            return C64HostInputPortValues {
                port_a: resolved_port_a,
                port_b: resolved_port_b,
            };
        }

        let (components, component_count) = self.connected_components();
        for component in &components[..component_count] {
            let low_port_a = component.columns & !resolved_port_a;
            let low_port_b = component.rows & !resolved_port_b;
            if low_port_b != 0 {
                resolved_port_a &= !component.columns;
                resolved_port_b &= !component.rows;
                continue;
            }
            if low_port_a == 0 {
                continue;
            }

            resolved_port_a &= !component.columns;
            resolved_port_b &= !(component.rows & !port_b.data_direction);
            for row in 0..C64_KEYBOARD_MATRIX_SIDE {
                let row_mask = 1_u8 << row;
                if component.rows & row_mask & port_b.data_direction & resolved_port_b == 0 {
                    continue;
                }
                if self.can_port_a_low_overpower_port_b_high(row, resolved_port_a) {
                    resolved_port_b &= !row_mask;
                }
            }
        }

        C64HostInputPortValues {
            port_a: resolved_port_a,
            port_b: resolved_port_b,
        }
    }

    pub fn clock_cycle(&mut self) {
        if self.restore_pulse_cycles_remaining == 0 {
            return;
        }
        self.restore_pulse_cycles_remaining = self.restore_pulse_cycles_remaining.saturating_sub(1);
    }

    /// RESET 清除主板单稳态电路；外部键盘和操纵杆的物理位置保持不变。
    pub fn reset_restore_circuit(&mut self) {
        self.restore_key_pressed = false;
        self.restore_pulse_cycles_remaining = 0;
    }

    fn set_restore_key_pressed(&mut self, pressed: bool) {
        if pressed && !self.restore_key_pressed && self.restore_pulse_cycles_remaining == 0 {
            self.restore_pulse_cycles_remaining = RESTORE_NMI_PULSE_CYCLES;
        }
        self.restore_key_pressed = pressed;
    }

    fn validate_joystick_mask(port: u8, value: u8) -> Result<(), C64HostInputError> {
        if value & !C64_CONTROL_PORT_DIGITAL_MASK != 0 {
            return Err(C64HostInputError::JoystickMask { port, value });
        }
        Ok(())
    }

    fn connected_components(&self) -> ([MatrixComponent; C64_KEYBOARD_MATRIX_SIDE], usize) {
        let mut components = [MatrixComponent::EMPTY; C64_KEYBOARD_MATRIX_SIDE];
        let mut component_count = 0;
        let mut visited_columns = 0_u8;
        for start_column in 0..C64_KEYBOARD_MATRIX_SIDE {
            let start_mask = 1_u8 << start_column;
            if visited_columns & start_mask != 0 || self.pressed_rows_by_column[start_column] == 0 {
                continue;
            }

            let mut component = MatrixComponent {
                columns: start_mask,
                rows: 0,
            };
            loop {
                let previous = component;
                for column in 0..C64_KEYBOARD_MATRIX_SIDE {
                    if component.columns & (1_u8 << column) != 0 {
                        component.rows |= self.pressed_rows_by_column[column];
                    }
                }
                for column in 0..C64_KEYBOARD_MATRIX_SIDE {
                    if self.pressed_rows_by_column[column] & component.rows != 0 {
                        component.columns |= 1_u8 << column;
                    }
                }
                if component == previous {
                    break;
                }
            }
            visited_columns |= component.columns;
            components[component_count] = component;
            component_count += 1;
        }
        (components, component_count)
    }

    fn can_port_a_low_overpower_port_b_high(&self, row: usize, port_a: u8) -> bool {
        let row_mask = 1_u8 << row;
        let grounded_connections = (0..C64_KEYBOARD_MATRIX_SIDE)
            .filter(|&column| {
                self.pressed_rows_by_column[column] & row_mask != 0
                    && port_a & (1_u8 << column) == 0
            })
            .count();
        if grounded_connections >= 2 {
            return true;
        }

        self.shift_lock_pressed
            && row == SHIFT_LOCK_ROW
            && port_a & (1_u8 << SHIFT_LOCK_COLUMN) == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MatrixComponent {
    columns: u8,
    rows: u8,
}

impl MatrixComponent {
    const EMPTY: Self = Self {
        columns: 0,
        rows: 0,
    };
}

#[cfg(test)]
mod tests {
    use super::{C64HostInput, C64HostInputPortState, C64HostInputPortValues};

    #[test]
    fn empty_keyboard_preserves_only_wired_port_levels() {
        let input = C64HostInput::new();
        assert_eq!(
            input.resolve_port_inputs(
                C64HostInputPortState {
                    data_direction: 0x0f,
                    external_input_pins: 0xf3,
                    output_pins: 0x5f,
                },
                C64HostInputPortState {
                    data_direction: 0xf0,
                    external_input_pins: 0xcf,
                    output_pins: 0xfa,
                },
            ),
            C64HostInputPortValues {
                port_a: 0x53,
                port_b: 0xca,
            }
        );
    }
}
