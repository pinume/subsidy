use rust_decimal::{Decimal, RoundingStrategy};

/// 保留两位小数，逢五进一，与 Excel `ROUND` 一致（默认的银行家舍入会与财务手算差 1 分）。
pub fn round_cents(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// 第 7.3/8.4/10.7 节的浮点尾差规则：与最近两位小数的差不超过`0.000001`时，
/// 返回该两位小数值；否则返回`None`，由调用方按数据异常终止处理并报告上下文。
pub fn to_cents(value: Decimal) -> Option<Decimal> {
    let rounded = round_cents(value);
    let tolerance = Decimal::new(1, 6); // 0.000001
    if (value - rounded).abs() <= tolerance {
        Some(rounded)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use rust_decimal::prelude::FromPrimitive;

    fn dec(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    #[test]
    fn to_cents_accepts_exact_value() {
        assert_eq!(to_cents(dec("99.50")), Some(dec("99.50")));
    }

    #[test]
    fn to_cents_accepts_within_tolerance() {
        // cleaning-rules.md 10.4 节示例尾差值；作为 f64 字面量时与 299.85 是同一个比特模式。
        let value = Decimal::from_f64(299.85).unwrap();
        assert_eq!(to_cents(value), Some(dec("299.85")));
    }

    #[test]
    fn round_cents_rounds_half_away_from_zero() {
        assert_eq!(round_cents(dec("14.985")), dec("14.99"));
        assert_eq!(round_cents(dec("-14.985")), dec("-14.99"));
        assert_eq!(round_cents(dec("14.984")), dec("14.98"));
    }

    #[test]
    fn to_cents_rejects_beyond_tolerance() {
        assert_eq!(to_cents(dec("99.505")), None);
    }
}
