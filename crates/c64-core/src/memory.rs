// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 连贯基础内存与物理页版本
//
//   文件:       memory.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

pub const BASE_RAM_BYTES: usize = 65_536;
pub const PAGE_BYTES: usize = 256;
pub const BASE_PAGE_COUNT: usize = BASE_RAM_BYTES / PAGE_BYTES;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PageDomain {
    #[default]
    Fast,
    BusBridge,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PhysicalTarget {
    #[default]
    BaseRam,
    BasicRom,
    KernalRom,
    CharacterRom,
    ProcessorPort,
    Vic,
    Sid,
    ColorRam,
    Cia1,
    Cia2,
    Cartridge,
    Reu,
    EnhancedDma,
    OpenBus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageDescriptor {
    pub domain: PageDomain,
    pub target: PhysicalTarget,
    pub side_effectful_reads: bool,
}

impl PageDescriptor {
    pub const FAST_RAM: Self = Self {
        domain: PageDomain::Fast,
        target: PhysicalTarget::BaseRam,
        side_effectful_reads: false,
    };

    pub const fn bridged(target: PhysicalTarget, side_effectful_reads: bool) -> Self {
        Self {
            domain: PageDomain::BusBridge,
            target,
            side_effectful_reads,
        }
    }

    pub const fn mapped_fast(target: PhysicalTarget) -> Self {
        Self {
            domain: PageDomain::Fast,
            target,
            side_effectful_reads: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PageMetadata {
    read_descriptor: PageDescriptor,
    write_descriptor: PageDescriptor,
    code_generation: u64,
}

impl PageMetadata {
    const INITIAL: Self = Self {
        read_descriptor: PageDescriptor::FAST_RAM,
        write_descriptor: PageDescriptor::FAST_RAM,
        code_generation: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryWriteSource {
    Cpu,
    Reu,
    EnhancedDma,
    HostLoader,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoherentMemory {
    base_ram: Box<[u8; BASE_RAM_BYTES]>,
    pages: [PageMetadata; BASE_PAGE_COUNT],
    mapping_generation: u64,
    memory_generation: u64,
}

impl Default for CoherentMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl CoherentMemory {
    pub fn new() -> Self {
        let mut memory = Self {
            base_ram: zeroed_base_ram(),
            pages: [PageMetadata::INITIAL; BASE_PAGE_COUNT],
            mapping_generation: 0,
            memory_generation: 0,
        };
        memory.install_default_c64_bus_pages();
        memory
    }

    fn install_default_c64_bus_pages(&mut self) {
        for page in 0xd0..=0xdf {
            let descriptor = PageDescriptor::bridged(
                match page {
                    0xd0..=0xd3 => PhysicalTarget::Vic,
                    0xd4..=0xd7 => PhysicalTarget::Sid,
                    0xd8..=0xdb => PhysicalTarget::ColorRam,
                    0xdc => PhysicalTarget::Cia1,
                    0xdd => PhysicalTarget::Cia2,
                    _ => PhysicalTarget::Cartridge,
                },
                true,
            );
            self.pages[page].read_descriptor = descriptor;
            self.pages[page].write_descriptor = descriptor;
        }
    }

    pub fn classify(&self, address: u16) -> PageDescriptor {
        self.classify_read(address)
    }

    pub fn classify_read(&self, address: u16) -> PageDescriptor {
        if address <= 1 {
            return PageDescriptor::bridged(PhysicalTarget::ProcessorPort, true);
        }
        self.pages[usize::from(address >> 8)].read_descriptor
    }

    pub fn classify_write(&self, address: u16) -> PageDescriptor {
        if address <= 1 {
            return PageDescriptor::bridged(PhysicalTarget::ProcessorPort, false);
        }
        self.pages[usize::from(address >> 8)].write_descriptor
    }

    pub fn set_page_descriptor(&mut self, page: u8, descriptor: PageDescriptor) {
        self.set_page_descriptors(page, descriptor, descriptor);
    }

    pub fn set_page_descriptors(
        &mut self,
        page: u8,
        read_descriptor: PageDescriptor,
        write_descriptor: PageDescriptor,
    ) {
        let metadata = &mut self.pages[usize::from(page)];
        if metadata.read_descriptor == read_descriptor
            && metadata.write_descriptor == write_descriptor
        {
            return;
        }
        metadata.read_descriptor = read_descriptor;
        metadata.write_descriptor = write_descriptor;
        self.mapping_generation = self.mapping_generation.wrapping_add(1);
    }

    pub(crate) fn apply_page_descriptors(
        &mut self,
        descriptors: &[(PageDescriptor, PageDescriptor); BASE_PAGE_COUNT],
    ) {
        let mut changed = false;
        for (metadata, &(read_descriptor, write_descriptor)) in
            self.pages.iter_mut().zip(descriptors)
        {
            if metadata.read_descriptor != read_descriptor
                || metadata.write_descriptor != write_descriptor
            {
                metadata.read_descriptor = read_descriptor;
                metadata.write_descriptor = write_descriptor;
                changed = true;
            }
        }
        if changed {
            self.mapping_generation = self.mapping_generation.wrapping_add(1);
        }
    }

    pub fn read_base_ram(&self, address: u16) -> u8 {
        self.base_ram[usize::from(address)]
    }

    pub fn write_base_ram(&mut self, address: u16, value: u8, _source: MemoryWriteSource) {
        self.base_ram[usize::from(address)] = value;
        let metadata = &mut self.pages[usize::from(address >> 8)];
        metadata.code_generation = metadata.code_generation.wrapping_add(1);
        self.memory_generation = self.memory_generation.wrapping_add(1);
    }

    pub fn code_generation(&self, page: u8) -> u64 {
        self.pages[usize::from(page)].code_generation
    }

    pub const fn mapping_generation(&self) -> u64 {
        self.mapping_generation
    }

    pub const fn memory_generation(&self) -> u64 {
        self.memory_generation
    }

    pub fn base_ram(&self) -> &[u8; BASE_RAM_BYTES] {
        &self.base_ram
    }

    pub(crate) fn clone_base_ram(&self) -> Box<[u8; BASE_RAM_BYTES]> {
        self.base_ram.clone()
    }

    pub(crate) fn restore_base_ram(&mut self, ram: &[u8]) {
        self.base_ram.copy_from_slice(ram);
        self.mapping_generation = self.mapping_generation.wrapping_add(1);
        self.memory_generation = self.memory_generation.wrapping_add(1);
        for page in &mut self.pages {
            page.code_generation = page.code_generation.wrapping_add(1);
        }
    }
}

fn zeroed_base_ram() -> Box<[u8; BASE_RAM_BYTES]> {
    vec![0; BASE_RAM_BYTES]
        .into_boxed_slice()
        .try_into()
        .expect("固定 64 KiB RAM 分配必须具有精确长度")
}

#[cfg(test)]
mod tests {
    use super::{CoherentMemory, MemoryWriteSource, PageDomain, PhysicalTarget};

    #[test]
    fn every_writer_participates_in_the_same_page_generation() {
        let mut memory = CoherentMemory::new();
        for (index, source) in [
            MemoryWriteSource::Cpu,
            MemoryWriteSource::Reu,
            MemoryWriteSource::EnhancedDma,
            MemoryWriteSource::HostLoader,
        ]
        .into_iter()
        .enumerate()
        {
            memory.write_base_ram(0x4000, u8::try_from(index).unwrap(), source);
        }
        assert_eq!(memory.code_generation(0x40), 4);
        assert_eq!(memory.memory_generation(), 4);
        assert_eq!(memory.read_base_ram(0x4000), 3);
    }

    #[test]
    fn processor_port_and_default_io_pages_are_bridged() {
        let memory = CoherentMemory::new();
        assert_eq!(
            memory.classify(0x0001).target,
            PhysicalTarget::ProcessorPort
        );
        assert_eq!(memory.classify(0xd020).target, PhysicalTarget::Vic);
        assert_eq!(memory.classify(0xd800).target, PhysicalTarget::ColorRam);
        assert_eq!(memory.classify(0xdc00).target, PhysicalTarget::Cia1);
        assert_eq!(memory.classify(0x4000).domain, PageDomain::Fast);
    }
}
