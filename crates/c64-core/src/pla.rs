// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - C64 PLA 地址译码
//
//   文件:       pla.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

const PROCESSOR_PORT_BANK_MASK: u8 = 0x07;
const CARTRIDGE_GAME_CONFIGURATION_BIT: u8 = 1 << 4;
const CARTRIDGE_EXROM_CONFIGURATION_BIT: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaInputs {
    pub exrom_line_high: bool,
    pub game_line_high: bool,
    pub processor_port: u8,
}

impl Default for PlaInputs {
    fn default() -> Self {
        Self {
            exrom_line_high: true,
            game_line_high: true,
            processor_port: 0x07,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CartridgeMode {
    #[default]
    Detached,
    Game8K,
    Game16K,
    Ultimax,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum PlaTarget {
    #[default]
    Ram,
    BasicRom,
    KernalRom,
    CharacterRom,
    Io,
    CartridgeLow,
    CartridgeHigh,
    OpenBus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C64Pla {
    read_map: [PlaTarget; 256],
    write_map: [PlaTarget; 256],
    configuration_code: u8,
    cartridge_mode: CartridgeMode,
}

impl Default for C64Pla {
    fn default() -> Self {
        Self::new(PlaInputs::default())
    }
}

impl C64Pla {
    pub fn new(inputs: PlaInputs) -> Self {
        let mut pla = Self {
            read_map: [PlaTarget::Ram; 256],
            write_map: [PlaTarget::Ram; 256],
            configuration_code: 0,
            cartridge_mode: CartridgeMode::Detached,
        };
        pla.configure(inputs);
        pla
    }

    pub const fn configuration_code(&self) -> u8 {
        self.configuration_code
    }

    pub const fn cartridge_mode(&self) -> CartridgeMode {
        self.cartridge_mode
    }

    pub fn configure(&mut self, inputs: PlaInputs) {
        let processor_port = inputs.processor_port & PROCESSOR_PORT_BANK_MASK;
        self.configuration_code = configuration_code(inputs);
        self.cartridge_mode = cartridge_mode(inputs.game_line_high, inputs.exrom_line_high);
        self.read_map.fill(PlaTarget::Ram);
        self.write_map.fill(PlaTarget::Ram);

        if self.cartridge_mode == CartridgeMode::Ultimax {
            self.configure_ultimax_map();
            return;
        }

        let loram = processor_port & 0x01 != 0;
        let hiram = processor_port & 0x02 != 0;
        let charen = processor_port & 0x04 != 0;
        let rom_control_enabled = loram || hiram;
        let basic_and_rom_low_enabled = loram && hiram;

        if basic_and_rom_low_enabled
            && matches!(
                self.cartridge_mode,
                CartridgeMode::Game8K | CartridgeMode::Game16K
            )
        {
            map_pages(&mut self.read_map, 0x80, 0x9f, PlaTarget::CartridgeLow);
        }

        if self.cartridge_mode == CartridgeMode::Game16K && hiram {
            map_pages(&mut self.read_map, 0xa0, 0xbf, PlaTarget::CartridgeHigh);
        } else if basic_and_rom_low_enabled {
            map_pages(&mut self.read_map, 0xa0, 0xbf, PlaTarget::BasicRom);
        }

        if rom_control_enabled {
            let d000_target = if charen {
                PlaTarget::Io
            } else {
                PlaTarget::CharacterRom
            };
            map_pages(&mut self.read_map, 0xd0, 0xdf, d000_target);
            if charen {
                map_pages(&mut self.write_map, 0xd0, 0xdf, PlaTarget::Io);
            }
        }

        if hiram {
            map_pages(&mut self.read_map, 0xe0, 0xff, PlaTarget::KernalRom);
        }
    }

    pub fn read_target(&self, address: u16) -> PlaTarget {
        self.read_map[usize::from(address >> 8)]
    }

    pub fn write_target(&self, address: u16) -> PlaTarget {
        self.write_map[usize::from(address >> 8)]
    }

    pub fn read_target_for_page(&self, page: u8) -> PlaTarget {
        self.read_map[usize::from(page)]
    }

    pub fn write_target_for_page(&self, page: u8) -> PlaTarget {
        self.write_map[usize::from(page)]
    }

    fn configure_ultimax_map(&mut self) {
        map_read_write(
            &mut self.read_map,
            &mut self.write_map,
            0x10,
            0x7f,
            PlaTarget::OpenBus,
        );
        map_read_write(
            &mut self.read_map,
            &mut self.write_map,
            0x80,
            0x9f,
            PlaTarget::CartridgeLow,
        );
        map_read_write(
            &mut self.read_map,
            &mut self.write_map,
            0xa0,
            0xcf,
            PlaTarget::OpenBus,
        );
        map_read_write(
            &mut self.read_map,
            &mut self.write_map,
            0xd0,
            0xdf,
            PlaTarget::Io,
        );
        map_read_write(
            &mut self.read_map,
            &mut self.write_map,
            0xe0,
            0xff,
            PlaTarget::CartridgeHigh,
        );
    }
}

pub const fn configuration_code(inputs: PlaInputs) -> u8 {
    (if inputs.game_line_high {
        0
    } else {
        CARTRIDGE_GAME_CONFIGURATION_BIT
    }) | (if inputs.exrom_line_high {
        0
    } else {
        CARTRIDGE_EXROM_CONFIGURATION_BIT
    }) | (inputs.processor_port & PROCESSOR_PORT_BANK_MASK)
}

pub const fn cartridge_mode(game_line_high: bool, exrom_line_high: bool) -> CartridgeMode {
    if game_line_high && exrom_line_high {
        CartridgeMode::Detached
    } else if game_line_high {
        CartridgeMode::Game8K
    } else if !exrom_line_high {
        CartridgeMode::Game16K
    } else {
        CartridgeMode::Ultimax
    }
}

fn map_pages(map: &mut [PlaTarget; 256], first_page: u8, last_page: u8, target: PlaTarget) {
    for page in first_page..=last_page {
        map[usize::from(page)] = target;
    }
}

fn map_read_write(
    read_map: &mut [PlaTarget; 256],
    write_map: &mut [PlaTarget; 256],
    first_page: u8,
    last_page: u8,
    target: PlaTarget,
) {
    map_pages(read_map, first_page, last_page, target);
    map_pages(write_map, first_page, last_page, target);
}

#[cfg(test)]
mod tests {
    use super::{C64Pla, CartridgeMode, PlaInputs, PlaTarget, configuration_code};

    const EXPECTED_READ_WINDOWS: [[PlaTarget; 4]; 32] = [
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::CharacterRom,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::BasicRom,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Io,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::BasicRom,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::CharacterRom,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::BasicRom,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Io,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::BasicRom,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::OpenBus,
            PlaTarget::Io,
            PlaTarget::CartridgeHigh,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::CharacterRom,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::CartridgeHigh,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::CartridgeHigh,
            PlaTarget::CharacterRom,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::Ram,
            PlaTarget::Io,
            PlaTarget::Ram,
        ],
        [
            PlaTarget::Ram,
            PlaTarget::CartridgeHigh,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
        [
            PlaTarget::CartridgeLow,
            PlaTarget::CartridgeHigh,
            PlaTarget::Io,
            PlaTarget::KernalRom,
        ],
    ];

    const fn inputs_for_code(code: u8) -> PlaInputs {
        PlaInputs {
            exrom_line_high: code & 0x08 == 0,
            game_line_high: code & 0x10 == 0,
            processor_port: code & 0x07,
        }
    }

    #[test]
    fn every_cpu_window_matches_the_vice_truth_table() {
        for code in 0_u8..32 {
            let inputs = inputs_for_code(code);
            let pla = C64Pla::new(inputs);
            assert_eq!(configuration_code(inputs), code);
            assert_eq!(pla.configuration_code(), code);
            assert_eq!(
                [
                    pla.read_target(0x8000),
                    pla.read_target(0xa000),
                    pla.read_target(0xd000),
                    pla.read_target(0xe000),
                ],
                EXPECTED_READ_WINDOWS[usize::from(code)],
                "PLA configuration {code}"
            );
        }
    }

    #[test]
    fn writes_reach_hidden_ram_except_for_io_and_ultimax_holes() {
        for code in 0_u8..32 {
            let pla = C64Pla::new(inputs_for_code(code));
            let ultimax = (16..=23).contains(&code);
            assert_eq!(pla.write_target(0x0200), PlaTarget::Ram);
            assert_eq!(
                pla.write_target(0xd000),
                if ultimax || EXPECTED_READ_WINDOWS[usize::from(code)][2] == PlaTarget::Io {
                    PlaTarget::Io
                } else {
                    PlaTarget::Ram
                }
            );
            assert_eq!(
                pla.write_target(0xa000),
                if ultimax {
                    PlaTarget::OpenBus
                } else {
                    PlaTarget::Ram
                }
            );
        }
    }

    #[test]
    fn physical_cartridge_lines_have_unambiguous_modes() {
        assert_eq!(
            C64Pla::new(inputs_for_code(0)).cartridge_mode(),
            CartridgeMode::Detached
        );
        assert_eq!(
            C64Pla::new(inputs_for_code(8)).cartridge_mode(),
            CartridgeMode::Game8K
        );
        assert_eq!(
            C64Pla::new(inputs_for_code(16)).cartridge_mode(),
            CartridgeMode::Ultimax
        );
        assert_eq!(
            C64Pla::new(inputs_for_code(24)).cartridge_mode(),
            CartridgeMode::Game16K
        );
    }
}
