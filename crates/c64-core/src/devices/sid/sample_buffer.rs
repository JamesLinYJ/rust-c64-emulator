// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - bounded SID audio sample buffer
//
//   File:       sid/sample_buffer.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

/// Fixed-capacity overwrite-oldest ring. Samples are stored as IEEE-754 bits
/// so complete machine state remains reflexively comparable and serializable.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub(crate) struct SidSampleBuffer {
    values: Box<[u32]>,
    read_index: usize,
    write_index: usize,
    length: usize,
}

impl SidSampleBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        debug_assert_ne!(capacity, 0);
        Self {
            values: vec![0; capacity].into_boxed_slice(),
            read_index: 0,
            write_index: 0,
            length: 0,
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) fn state_is_valid(&self, expected_capacity: usize) -> bool {
        self.values.len() == expected_capacity
            && self.read_index < expected_capacity
            && self.write_index < expected_capacity
            && self.length <= expected_capacity
    }

    pub(crate) fn push(&mut self, value: f32) -> bool {
        let overflowed = self.length == self.values.len();
        self.values[self.write_index] = value.to_bits();
        self.write_index = (self.write_index + 1) % self.values.len();
        if overflowed {
            self.read_index = (self.read_index + 1) % self.values.len();
        } else {
            self.length += 1;
        }
        overflowed
    }

    pub(crate) fn drain(&mut self, maximum_length: usize) -> Vec<f32> {
        let count = self.length.min(maximum_length);
        let mut output = vec![0.0; count];
        self.pull_into(&mut output);
        output
    }

    pub(crate) fn pull_into(&mut self, destination: &mut [f32]) -> usize {
        let count = self.length.min(destination.len());
        for (offset, destination_sample) in destination.iter_mut().take(count).enumerate() {
            let source = (self.read_index + offset) % self.values.len();
            *destination_sample = f32::from_bits(self.values[source]);
        }
        self.read_index = (self.read_index + count) % self.values.len();
        self.length -= count;
        count
    }

    pub(crate) fn clear(&mut self) {
        self.read_index = 0;
        self.write_index = 0;
        self.length = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::SidSampleBuffer;

    #[test]
    fn overwrites_the_oldest_sample_without_allocating() {
        let mut buffer = SidSampleBuffer::new(3);
        assert!(!buffer.push(1.0));
        assert!(!buffer.push(2.0));
        assert!(!buffer.push(3.0));
        assert!(buffer.push(4.0));

        assert_eq!(buffer.drain(usize::MAX), vec![2.0, 3.0, 4.0]);
    }
}
