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
