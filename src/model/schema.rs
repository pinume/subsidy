/// 金额精度：区分“保留原精度”（第 5 节交易金额）与“固定两位小数”（第 5 节补贴金额、第 6–8、10 节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalScale {
    Original,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Text,
    Decimal(DecimalScale),
    Integer,
    /// 百分比数值，XLSX 显示格式 `0.00%`，底层保存十进制比例（如 0.15）。
    Ratio,
    Date,
    DateTime,
}

#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub name: &'static str,
    pub ty: ColumnType,
}
