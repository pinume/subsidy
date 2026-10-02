/// 整行填色：黄色用于重复记录，粉色用于异常/退货/沉底记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// `#FFEB9C`
    Yellow,
    /// `#FFC7CE`
    Pink,
}
