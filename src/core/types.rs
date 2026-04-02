#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuLimit(u32);

impl CpuLimit {
    pub fn from_centi_cores(value: u32) -> Self {
        Self(value)
    }

    pub fn centi_cores(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimit(u64);

impl MemoryLimit {
    pub fn from_bytes(value: u64) -> Self {
        Self(value)
    }

    pub fn bytes(self) -> u64 {
        self.0
    }
}
