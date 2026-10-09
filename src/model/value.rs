use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;

/// 输出单元格值；类型按单元格记录，允许同一列中出现 `Empty` 与该列类型的值并存
/// （以及第 8.4 节 `其他支付` 列中数值与文本 `-` 并存）。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Text(String),
    Decimal(Decimal),
    Integer(i64),
    Ratio(Decimal),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

impl From<crate::utils::doc_no::MatchDocNo> for Value {
    fn from(doc: crate::utils::doc_no::MatchDocNo) -> Self {
        Value::Text(doc.into_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::doc_no::MatchDocNo;

    #[test]
    fn converts_match_doc_no_to_text_value() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let doc = MatchDocNo::from_date_and_code(date, "ZFFX000003").unwrap();
        let val: Value = doc.into();
        assert_eq!(val, Value::Text("260101ZFFX000003".to_string()));
    }
}
