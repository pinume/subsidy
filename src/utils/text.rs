/// f64 能精确表示的最大整数（2^53）；超出后无法保证按原样还原标识字段。
const MAX_EXACT_INT_F64: f64 = 9_007_199_254_740_992.0;

/// 把 Excel 误存为数值的标识字段（如订单号）还原为完整整数文本；
/// 带小数或超出 f64 精度范围（无法保证无损还原）时返回`None`。
pub fn identifier_from_float(value: f64) -> Option<String> {
    if !value.is_finite() || value.fract() != 0.0 || value.abs() > MAX_EXACT_INT_F64 {
        return None;
    }
    Some(format!("{value:.0}"))
}

/// 检查匹配片段前后字符：若紧邻数字或 ASCII 字母，则视为从更长的数字/字母串中截取，
/// 返回`false`。用于弥补`regex`不支持环视的限制（先匹配候选，再校验边界）。
pub fn has_isolated_boundaries(haystack: &str, start: usize, end: usize) -> bool {
    let before_ok = haystack[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_ascii_alphanumeric());
    let after_ok = haystack[end..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric());
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_from_float_reconstructs_integer() {
        assert_eq!(
            identifier_from_float(20260914123456.0),
            Some("20260914123456".to_string())
        );
    }

    #[test]
    fn identifier_from_float_rejects_fraction() {
        assert_eq!(identifier_from_float(20260914.5), None);
    }

    #[test]
    fn identifier_from_float_rejects_precision_loss() {
        assert_eq!(identifier_from_float(MAX_EXACT_INT_F64 + 2.0), None);
    }

    #[test]
    fn boundary_check_rejects_substring_of_longer_run() {
        let haystack = "参考号16867252734N88";
        let start = haystack.find("16867252734N").unwrap();
        let end = start + "16867252734N".len();
        assert!(!has_isolated_boundaries(haystack, start, end));
    }

    #[test]
    fn boundary_check_accepts_isolated_match() {
        let haystack = "参考号：16867252734N。";
        let start = haystack.find("16867252734N").unwrap();
        let end = start + "16867252734N".len();
        assert!(has_isolated_boundaries(haystack, start, end));
    }
}
