use std::time::SystemTime;

use crate::core::{OutputFrame, OutputStream};

pub fn next_sequence(current: &mut u64) -> u64 {
    let sequence = *current;
    *current += 1;
    sequence
}

#[derive(Debug, Clone, Default)]
pub struct OutputCollector {
    next_sequence: u64,
}

impl OutputCollector {
    pub fn push_stdout(&mut self, bytes: impl AsRef<[u8]>) -> OutputFrame {
        self.push(OutputStream::Stdout, bytes)
    }

    pub fn push_stderr(&mut self, bytes: impl AsRef<[u8]>) -> OutputFrame {
        self.push(OutputStream::Stderr, bytes)
    }

    pub fn push_pty(&mut self, bytes: impl AsRef<[u8]>) -> OutputFrame {
        self.push(OutputStream::PtyMerged, bytes)
    }

    fn push(&mut self, stream: OutputStream, bytes: impl AsRef<[u8]>) -> OutputFrame {
        OutputFrame {
            sequence: next_sequence(&mut self.next_sequence),
            captured_at: SystemTime::now(),
            stream,
            bytes: bytes.as_ref().to_vec(),
        }
    }
}
