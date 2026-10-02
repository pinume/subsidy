use chrono::NaiveDate;

/// 删除开头的"收款"二字；没有该前缀时直接返回原值。
pub fn strip_receipt_prefix(value: &str) -> &str {
    value.strip_prefix("收款").unwrap_or(value)
}

/// 生成`匹配单据号`：日期转换为`yymmdd`后，与去前缀的单据号直接拼接，不加分隔符。
pub fn build_match_doc_no(date: NaiveDate, document_no: &str) -> String {
    format!(
        "{}{}",
        date.format("%y%m%d"),
        strip_receipt_prefix(document_no)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_receipt_prefix_when_present() {
        assert_eq!(strip_receipt_prefix("收款ZFFX000003"), "ZFFX000003");
    }

    #[test]
    fn keeps_value_without_prefix() {
        assert_eq!(strip_receipt_prefix("ZFFX000003"), "ZFFX000003");
    }

    #[test]
    fn builds_match_doc_no() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(
            build_match_doc_no(date, "收款ZFFX000003"),
            "260101ZFFX000003"
        );
    }
}
