use anyhow::{Result, anyhow, bail};
use rust_decimal::{Decimal, RoundingStrategy};
use std::str::FromStr;

pub use crate::core::types::{CpuLimit, MemoryLimit};

const BYTES_PER_KIB: u64 = 1024;
const BYTES_PER_MIB: u64 = BYTES_PER_KIB * 1024;
const BYTES_PER_GIB: u64 = BYTES_PER_MIB * 1024;
const BYTES_PER_TIB: u64 = BYTES_PER_GIB * 1024;

impl CpuLimit {
    pub fn parse(raw: &str) -> Result<Self> {
        let normalized = raw.to_ascii_lowercase();
        let number = normalized
            .strip_suffix('c')
            .ok_or_else(|| anyhow!("cpu limit must end with 'c'"))?;
        let centi_cores = parse_rounded_positive(number, Decimal::from(100u32), "cpu limit")?;
        let centi_cores =
            u32::try_from(centi_cores).map_err(|_| anyhow!("cpu limit exceeds supported range"))?;

        if centi_cores == 0 {
            bail!("cpu limit must be at least 1 centi-core");
        }

        Ok(Self::from_centi_cores(centi_cores))
    }
}

impl MemoryLimit {
    pub fn parse(raw: &str) -> Result<Self> {
        let normalized = raw.to_ascii_lowercase();
        let (number, unit) = normalized.split_at(
            normalized
                .len()
                .checked_sub(1)
                .ok_or_else(|| anyhow!("memory limit requires a value and unit"))?,
        );

        let multiplier = match unit {
            "b" => Decimal::from(1u32),
            "k" => Decimal::from(BYTES_PER_KIB),
            "m" => Decimal::from(BYTES_PER_MIB),
            "g" => Decimal::from(BYTES_PER_GIB),
            "t" => Decimal::from(BYTES_PER_TIB),
            _ => bail!("memory limit unit must be one of b, k, m, g, t"),
        };

        let bytes = parse_rounded_positive(number, multiplier, "memory limit")?;
        if bytes < BYTES_PER_MIB {
            bail!("memory limit must be at least 1 MiB");
        }

        Ok(Self::from_bytes(bytes))
    }
}

pub fn parse_cpu_limit(raw: &str) -> Result<CpuLimit> {
    CpuLimit::parse(raw)
}

pub fn parse_memory_limit(raw: &str) -> Result<MemoryLimit> {
    MemoryLimit::parse(raw)
}

fn parse_rounded_positive(number: &str, scale: Decimal, label: &str) -> Result<u64> {
    if number.is_empty() {
        bail!("{label} requires a numeric value");
    }

    if number.contains('_') || number.contains('e') || number.contains('E') {
        bail!("{label} must use plain decimal notation");
    }

    let value = Decimal::from_str(number).map_err(|_| anyhow!("{label} is not a valid decimal"))?;
    if value <= Decimal::ZERO {
        bail!("{label} must be greater than zero");
    }

    let scaled = value
        .checked_mul(scale)
        .ok_or_else(|| anyhow!("{label} exceeds supported range"))?;
    let rounded = scaled.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    let rounded = rounded.trunc();

    rounded
        .to_string()
        .parse::<u64>()
        .map_err(|_| anyhow!("{label} exceeds supported range"))
}
