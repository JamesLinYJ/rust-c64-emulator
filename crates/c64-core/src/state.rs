// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 版本化架构状态格式
//
//   文件:       state.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::{
    architecture::{
        CoreConfig, ExecutionConfigError, ExecutionController, ExecutionRequest, MachineProfile,
        PacingMode, SlotsPerSystemCycle, TurboSpeedRequest, VideoStandard,
    },
    clock::VirtualTimestamp,
    cpu::Cpu6510State,
    devices::sid::SidModel,
    machine::C64Core,
    memory::BASE_RAM_BYTES,
};

const LEGACY_SAVE_STATE_MAGIC: [u8; 8] = *b"RC64VM01";
const SAVE_STATE_MAGIC: [u8; 8] = *b"RC64VM02";
pub const SAVE_STATE_FORMAT_VERSION: u16 = 2;
const SAVE_STATE_HEADER_BYTES: usize = 18;
const MAXIMUM_SAVE_STATE_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
#[cfg(test)]
const SECTION_COUNT: u16 = 4;
const SECTION_CONFIGURATION: [u8; 4] = *b"CONF";
const SECTION_TIME: [u8; 4] = *b"TIME";
const SECTION_CPU: [u8; 4] = *b"CPU0";
const SECTION_RAM: [u8; 4] = *b"RAM0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyStateImage {
    pub config: CoreConfig,
    pub execution: ExecutionController,
    pub timestamp: VirtualTimestamp,
    pub cpu: Cpu6510State,
    pub base_ram: Box<[u8; BASE_RAM_BYTES]>,
}

pub(crate) enum DecodedState {
    Full(Box<C64Core>),
    Legacy(LegacyStateImage),
}

pub(crate) fn encode(core: &C64Core) -> Vec<u8> {
    let config = wincode::config::Configuration::default()
        .with_preallocation_size_limit::<MAXIMUM_SAVE_STATE_PAYLOAD_BYTES>();
    let payload = wincode::config::serialize(core, config)
        .expect("a validated C64 state must fit the save-state codec limits");
    assert!(
        payload.len() <= MAXIMUM_SAVE_STATE_PAYLOAD_BYTES,
        "a C64 save-state must fit the declared payload limit"
    );

    let mut output = Vec::with_capacity(SAVE_STATE_HEADER_BYTES + payload.len());
    output.extend_from_slice(&SAVE_STATE_MAGIC);
    output.extend_from_slice(&SAVE_STATE_FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("the bounded save-state payload length must fit u32")
            .to_le_bytes(),
    );
    output.extend_from_slice(&crc32(&payload).to_le_bytes());
    output.extend_from_slice(&payload);
    output
}

#[cfg(test)]
fn encode_legacy(image: &LegacyStateImage) -> Vec<u8> {
    let mut output = Vec::with_capacity(BASE_RAM_BYTES + 96);
    output.extend_from_slice(&LEGACY_SAVE_STATE_MAGIC);
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&SECTION_COUNT.to_le_bytes());

    let status = image.execution.status();
    let (request_code, requested_slots) = match status.requested {
        ExecutionRequest::Strict => (0, 1),
        ExecutionRequest::Turbo(TurboSpeedRequest::Manual(slots)) => (1, slots.get()),
        ExecutionRequest::Turbo(TurboSpeedRequest::Auto { maximum }) => (2, maximum.get()),
    };
    append_section(
        &mut output,
        SECTION_CONFIGURATION,
        &[
            image.config.profile.code(),
            image.config.video_standard.code(),
            image.config.pacing.code(),
            request_code,
            requested_slots,
            status.effective_slots.get(),
            u8::from(status.awaiting_auto_calibration),
            image.config.sid_model.code(),
        ],
    );

    let mut time = [0_u8; 9];
    time[..8].copy_from_slice(&image.timestamp.system_cycle.to_le_bytes());
    time[8] = image.timestamp.slot;
    append_section(&mut output, SECTION_TIME, &time);

    let mut cpu = [0_u8; 8];
    cpu[0] = image.cpu.accumulator;
    cpu[1] = image.cpu.index_x;
    cpu[2] = image.cpu.index_y;
    cpu[3] = image.cpu.stack_pointer;
    cpu[4] = image.cpu.status;
    cpu[5..7].copy_from_slice(&image.cpu.program_counter.to_le_bytes());
    append_section(&mut output, SECTION_CPU, &cpu);
    append_section(&mut output, SECTION_RAM, image.base_ram.as_ref());
    output
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedState, StateError> {
    if bytes.len() < 8 {
        return Err(StateError::TruncatedHeader);
    }
    if bytes[..8] == SAVE_STATE_MAGIC {
        return decode_full(bytes).map(Box::new).map(DecodedState::Full);
    }
    if bytes[..8] == LEGACY_SAVE_STATE_MAGIC {
        return decode_legacy(bytes).map(DecodedState::Legacy);
    }
    Err(StateError::InvalidMagic)
}

fn decode_full(bytes: &[u8]) -> Result<C64Core, StateError> {
    if bytes.len() < SAVE_STATE_HEADER_BYTES {
        return Err(StateError::TruncatedHeader);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != SAVE_STATE_FORMAT_VERSION {
        return Err(StateError::UnsupportedVersion(version));
    }
    let payload_length = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if payload_length > MAXIMUM_SAVE_STATE_PAYLOAD_BYTES {
        return Err(StateError::PayloadTooLarge {
            actual: payload_length,
            maximum: MAXIMUM_SAVE_STATE_PAYLOAD_BYTES,
        });
    }
    let expected_length = SAVE_STATE_HEADER_BYTES
        .checked_add(payload_length)
        .ok_or(StateError::PayloadLengthOverflow)?;
    if bytes.len() != expected_length {
        return Err(StateError::PayloadLengthMismatch {
            expected: expected_length,
            actual: bytes.len(),
        });
    }
    let expected_checksum = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    let payload = &bytes[SAVE_STATE_HEADER_BYTES..];
    let actual_checksum = crc32(payload);
    if actual_checksum != expected_checksum {
        return Err(StateError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }
    let config = wincode::config::Configuration::default()
        .with_preallocation_size_limit::<MAXIMUM_SAVE_STATE_PAYLOAD_BYTES>();
    wincode::config::deserialize_exact(payload, config)
        .map_err(|error| StateError::Codec(error.to_string()))
}

fn decode_legacy(bytes: &[u8]) -> Result<LegacyStateImage, StateError> {
    if bytes.len() < 12 {
        return Err(StateError::TruncatedHeader);
    }
    if bytes[..8] != LEGACY_SAVE_STATE_MAGIC {
        return Err(StateError::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != 1 {
        return Err(StateError::UnsupportedVersion(version));
    }
    let section_count = usize::from(u16::from_le_bytes([bytes[10], bytes[11]]));
    let mut cursor = 12;
    let mut configuration = None;
    let mut time = None;
    let mut cpu = None;
    let mut ram = None;

    for _ in 0..section_count {
        if bytes.len().saturating_sub(cursor) < 8 {
            return Err(StateError::TruncatedSectionHeader);
        }
        let tag = [
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ];
        let length = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        cursor += 8;
        let end = cursor
            .checked_add(length)
            .ok_or(StateError::SectionLengthOverflow)?;
        let payload = bytes.get(cursor..end).ok_or(StateError::TruncatedSection)?;
        match tag {
            SECTION_CONFIGURATION => configuration = Some(payload),
            SECTION_TIME => time = Some(payload),
            SECTION_CPU => cpu = Some(payload),
            SECTION_RAM => ram = Some(payload),
            _ => {}
        }
        cursor = end;
    }

    let (config, execution) = decode_configuration(
        configuration.ok_or(StateError::MissingSection(SECTION_CONFIGURATION))?,
    )?;
    let timestamp = decode_time(time.ok_or(StateError::MissingSection(SECTION_TIME))?)?;
    if timestamp.slot >= execution.status().effective_slots.get() {
        return Err(StateError::InvalidClockSlot {
            slot: timestamp.slot,
            slots_per_system_cycle: execution.status().effective_slots.get(),
        });
    }
    let cpu = decode_cpu(cpu.ok_or(StateError::MissingSection(SECTION_CPU))?)?;
    let ram = ram.ok_or(StateError::MissingSection(SECTION_RAM))?;
    if ram.len() != BASE_RAM_BYTES {
        return Err(StateError::InvalidSectionLength {
            tag: SECTION_RAM,
            expected: BASE_RAM_BYTES,
            actual: ram.len(),
        });
    }
    let mut base_ram: Box<[u8; BASE_RAM_BYTES]> = vec![0; BASE_RAM_BYTES]
        .into_boxed_slice()
        .try_into()
        .expect("固定 64 KiB RAM 分配必须具有精确长度");
    base_ram.copy_from_slice(ram);

    Ok(LegacyStateImage {
        config,
        execution,
        timestamp,
        cpu,
        base_ram,
    })
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
fn append_section(output: &mut Vec<u8>, tag: [u8; 4], payload: &[u8]) {
    output.extend_from_slice(&tag);
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("save-state section must fit in a 32-bit length")
            .to_le_bytes(),
    );
    output.extend_from_slice(payload);
}

fn decode_configuration(payload: &[u8]) -> Result<(CoreConfig, ExecutionController), StateError> {
    require_length(SECTION_CONFIGURATION, payload, 8)?;
    let profile =
        MachineProfile::from_code(payload[0]).ok_or(StateError::InvalidProfile(payload[0]))?;
    let video_standard =
        VideoStandard::from_code(payload[1]).ok_or(StateError::InvalidVideoStandard(payload[1]))?;
    let pacing = PacingMode::from_code(payload[2]).ok_or(StateError::InvalidPacing(payload[2]))?;
    let sid_model =
        SidModel::from_code(payload[7]).ok_or(StateError::InvalidSidModel(payload[7]))?;
    let mut execution = ExecutionController::new();
    match payload[3] {
        0 => {
            if payload[5] != SlotsPerSystemCycle::STRICT.get() || payload[6] != 0 {
                return Err(StateError::InconsistentExecutionState);
            }
        }
        1 => {
            let request = TurboSpeedRequest::manual(payload[4])?;
            execution.request(ExecutionRequest::Turbo(request));
            if payload[5] != payload[4] || payload[6] != 0 {
                return Err(StateError::InconsistentExecutionState);
            }
        }
        2 => {
            let request = TurboSpeedRequest::automatic(payload[4])?;
            execution.request(ExecutionRequest::Turbo(request));
            match payload[6] {
                0 => {
                    let resolved = decode_slots(payload[5])?;
                    execution.lock_auto_slots(resolved)?;
                }
                1 if payload[5] == SlotsPerSystemCycle::STRICT.get() => {}
                _ => return Err(StateError::InconsistentExecutionState),
            }
        }
        code => return Err(StateError::InvalidExecutionRequest(code)),
    }
    Ok((
        CoreConfig {
            profile,
            video_standard,
            pacing,
            sid_model,
        },
        execution,
    ))
}

fn decode_slots(value: u8) -> Result<SlotsPerSystemCycle, StateError> {
    if value == SlotsPerSystemCycle::STRICT.get() {
        return Ok(SlotsPerSystemCycle::STRICT);
    }
    Ok(SlotsPerSystemCycle::try_turbo(value)?)
}

fn decode_time(payload: &[u8]) -> Result<VirtualTimestamp, StateError> {
    require_length(SECTION_TIME, payload, 9)?;
    let mut cycle_bytes = [0_u8; 8];
    cycle_bytes.copy_from_slice(&payload[..8]);
    Ok(VirtualTimestamp {
        system_cycle: u64::from_le_bytes(cycle_bytes),
        slot: payload[8],
    })
}

fn decode_cpu(payload: &[u8]) -> Result<Cpu6510State, StateError> {
    require_length(SECTION_CPU, payload, 8)?;
    let mut state = Cpu6510State {
        accumulator: payload[0],
        index_x: payload[1],
        index_y: payload[2],
        stack_pointer: payload[3],
        status: payload[4],
        program_counter: u16::from_le_bytes([payload[5], payload[6]]),
    };
    state.normalize_status();
    Ok(state)
}

fn require_length(tag: [u8; 4], payload: &[u8], expected: usize) -> Result<(), StateError> {
    if payload.len() == expected {
        return Ok(());
    }
    Err(StateError::InvalidSectionLength {
        tag,
        expected,
        actual: payload.len(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    TruncatedHeader,
    InvalidMagic,
    UnsupportedVersion(u16),
    TruncatedSectionHeader,
    TruncatedSection,
    SectionLengthOverflow,
    MissingSection([u8; 4]),
    InvalidSectionLength {
        tag: [u8; 4],
        expected: usize,
        actual: usize,
    },
    InvalidProfile(u8),
    InvalidVideoStandard(u8),
    InvalidPacing(u8),
    InvalidSidModel(u8),
    InvalidExecutionRequest(u8),
    InconsistentExecutionState,
    InvalidClockSlot {
        slot: u8,
        slots_per_system_cycle: u8,
    },
    PayloadTooLarge {
        actual: usize,
        maximum: usize,
    },
    PayloadLengthOverflow,
    PayloadLengthMismatch {
        expected: usize,
        actual: usize,
    },
    ChecksumMismatch {
        expected: u32,
        actual: u32,
    },
    Codec(String),
    InvalidState(&'static str),
    ExecutionConfiguration(ExecutionConfigError),
}

impl From<ExecutionConfigError> for StateError {
    fn from(error: ExecutionConfigError) -> Self {
        Self::ExecutionConfiguration(error)
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader => formatter.write_str("save-state 头部不完整"),
            Self::InvalidMagic => formatter.write_str("save-state magic 不匹配"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "不支持 save-state 格式版本 {version}")
            }
            Self::TruncatedSectionHeader => formatter.write_str("save-state section 头部不完整"),
            Self::TruncatedSection => formatter.write_str("save-state section 数据不完整"),
            Self::SectionLengthOverflow => formatter.write_str("save-state section 长度溢出"),
            Self::MissingSection(tag) => write!(formatter, "save-state 缺少 section {tag:?}"),
            Self::InvalidSectionLength {
                tag,
                expected,
                actual,
            } => write!(
                formatter,
                "save-state section {tag:?} 长度应为 {expected}，实际为 {actual}",
            ),
            Self::InvalidProfile(code) => write!(formatter, "无效 machine profile 编号 {code}"),
            Self::InvalidVideoStandard(code) => write!(formatter, "无效视频制式编号 {code}"),
            Self::InvalidPacing(code) => write!(formatter, "无效 pacing 编号 {code}"),
            Self::InvalidSidModel(code) => write!(formatter, "无效 SID model 编号 {code}"),
            Self::InvalidExecutionRequest(code) => {
                write!(formatter, "无效 execution request 编号 {code}")
            }
            Self::InconsistentExecutionState => {
                formatter.write_str("save-state execution 请求与有效档位不一致")
            }
            Self::InvalidClockSlot {
                slot,
                slots_per_system_cycle,
            } => write!(
                formatter,
                "虚拟时钟槽位 {slot} 超出每周期 {slots_per_system_cycle} 个槽位的范围",
            ),
            Self::PayloadTooLarge { actual, maximum } => write!(
                formatter,
                "save-state payload 为 {actual} 字节，超过 {maximum} 字节上限",
            ),
            Self::PayloadLengthOverflow => formatter.write_str("save-state payload 长度溢出"),
            Self::PayloadLengthMismatch { expected, actual } => write!(
                formatter,
                "save-state 总长度应为 {expected} 字节，实际为 {actual} 字节",
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                formatter,
                "save-state CRC32 应为 {expected:#010x}，实际为 {actual:#010x}",
            ),
            Self::Codec(error) => write!(formatter, "save-state payload 无效：{error}"),
            Self::InvalidState(reason) => write!(formatter, "save-state 硬件状态无效：{reason}"),
            Self::ExecutionConfiguration(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod tests {
    use crate::{C64Core, MemoryWriteSource};

    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn legacy_v1_images_remain_loadable() {
        let mut execution = ExecutionController::new();
        execution.request(ExecutionRequest::Turbo(
            TurboSpeedRequest::manual(4).unwrap(),
        ));
        let mut base_ram: Box<[u8; BASE_RAM_BYTES]> = vec![0; BASE_RAM_BYTES]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        base_ram[0xc000] = 0x5a;
        let image = LegacyStateImage {
            config: CoreConfig::default(),
            execution,
            timestamp: VirtualTimestamp {
                system_cycle: 123,
                slot: 2,
            },
            cpu: Cpu6510State {
                program_counter: 0x2000,
                ..Cpu6510State::default()
            },
            base_ram,
        };

        let mut core = C64Core::default();
        core.write_base_ram(0xc000, 0, MemoryWriteSource::HostLoader);
        core.load_state(&encode_legacy(&image)).unwrap();
        assert_eq!(core.config(), image.config);
        assert_eq!(core.execution_status(), image.execution.status());
        assert_eq!(core.timestamp(), image.timestamp);
        assert_eq!(core.cpu_state(), image.cpu);
        assert_eq!(core.read_base_ram(0xc000), 0x5a);
    }

    #[test]
    fn checksummed_noncanonical_payload_is_rejected_by_the_codec() {
        let mut encoded = encode(&C64Core::default());
        encoded[SAVE_STATE_HEADER_BYTES..SAVE_STATE_HEADER_BYTES + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        let checksum = crc32(&encoded[SAVE_STATE_HEADER_BYTES..]);
        encoded[14..18].copy_from_slice(&checksum.to_le_bytes());

        assert!(matches!(decode(&encoded), Err(StateError::Codec(_))));
    }
}
