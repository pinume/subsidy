use std::collections::HashSet;
use std::ops::Deref;
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

/// 匹配单据号值对象，由 6 位日期前缀 `yymmdd` 与规范化的单据号组成。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchDocNo(String);

impl MatchDocNo {
    /// 由日期与单据号构造（收款单第9.4节、销售用券第10.5节）：剥离“收款”前缀，空值返回 None。
    pub fn from_date_and_code(date: NaiveDate, raw_code: &str) -> Option<Self> {
        let stripped = strip_receipt_prefix(raw_code.trim());
        if stripped.is_empty() {
            return None;
        }
        Some(Self(format!("{}{}", date.format("%y%m%d"), stripped)))
    }

    /// 从备注文本中按正则提取唯一的销售日期与单据号，生成匹配单据号；
    /// 任一字段未命中或存在歧义（多个不同候选值）时返回`None`。
    pub fn from_remark(remark: &str) -> Option<Self> {
        let fixed = apply_known_remark_fixes(remark);

        let dates_found: HashSet<NaiveDate> = date_label_regex()
            .captures_iter(&fixed)
            .filter_map(|caps| parse_labeled_date(&caps[1], &caps[2], &caps[3]))
            .collect();
        if dates_found.len() != 1 {
            return None;
        }
        let date = *dates_found.iter().next().unwrap();

        let raw_doc_nos: HashSet<&str> = doc_no_label_regex()
            .captures_iter(&fixed)
            .map(|caps| caps.get(1).unwrap().as_str())
            .collect();
        if raw_doc_nos.len() != 1 {
            return None;
        }
        let raw_doc_no = *raw_doc_nos.iter().next().unwrap();
        let stripped = strip_receipt_prefix(raw_doc_no);
        let normalized = normalize_document_no(stripped)?;
        Some(Self(format!("{}{}", date.format("%y%m%d"), normalized)))
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// project.md 3.4 节已确认的定向修正，作用于整条备注文本。
fn apply_known_remark_fixes(remark: &str) -> String {
    remark
        .replace("2026-0101", "2026-01-01")
        .replace("202601-04", "2026-01-04")
        .replace("2026-26-29", "2026-06-29")
}

fn date_label_regex() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?:销售日期|购机日期)[：:、\s]*([0-9]{2,4})[-./年]([0-9]{1,2})[-./月]([0-9]{1,2})日?",
        )
        .unwrap()
    });
    &RE
}

fn doc_no_label_regex() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:单据号收款号|单据号收款|单据收款号|单据号)[：:、\s]*((?:收款)?[A-Za-z]*[0-9][A-Za-z0-9]*)").unwrap()
    });
    &RE
}

fn parse_labeled_date(year: &str, month: &str, day: &str) -> Option<NaiveDate> {
    let mut year: i32 = year.parse().ok()?;
    if year < 100 {
        year += 2000;
    }
    NaiveDate::from_ymd_opt(year, month.parse().ok()?, day.parse().ok()?)
}

impl Deref for MatchDocNo {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 标准化并校验单据号：10 位直接使用；11 位从首个补零段删除一个 0；
/// `ZFP300008`定向修正为`ZFP3000008`；其余情况判定无效。
fn normalize_document_no(raw: &str) -> Option<String> {
    match raw.len() {
        10 => Some(raw.to_string()),
        11 => {
            let zero = raw.find('0')?;
            let mut corrected = raw.to_string();
            corrected.remove(zero);
            Some(corrected)
        }
        9 if raw == "ZFP300008" => Some("ZFP3000008".to_string()),
        _ => None,
    }
}

/// 删除开头的"收款"二字；没有该前缀时直接返回原值。
fn strip_receipt_prefix(value: &str) -> &str {
    value.strip_prefix("收款").unwrap_or(value)
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
    fn constructs_match_doc_no_from_date_and_code() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let doc = MatchDocNo::from_date_and_code(date, "收款ZFFX000003").unwrap();
        assert_eq!(&*doc, "260101ZFFX000003");

        // 收款单第9.4节与销售用券第10.5节：保留原始单据号除“收款”前缀外的原值，不进行 11 位去零
        let doc11 = MatchDocNo::from_date_and_code(date, "收款0023000002").unwrap();
        assert_eq!(&*doc11, "2601010023000002");

        // 空单据号返回 None
        assert!(MatchDocNo::from_date_and_code(date, "").is_none());
    }

    #[test]
    fn normalizes_document_no_lengths() {
        assert_eq!(
            normalize_document_no("ZEXQ000062"),
            Some("ZEXQ000062".to_string())
        );
        assert_eq!(
            normalize_document_no("ZHLT0000524"),
            Some("ZHLT000524".to_string())
        );
        assert_eq!(
            normalize_document_no("ZFTY0000027"),
            Some("ZFTY000027".to_string())
        );
        assert_eq!(
            normalize_document_no("ZFP300008"),
            Some("ZFP3000008".to_string())
        );
        assert_eq!(normalize_document_no("ZFP300009"), None); // 其他 9 位内容判定无效
        assert_eq!(normalize_document_no("ZABC12345678"), None); // 12 位无法处理
    }

    #[test]
    fn parses_match_doc_no_from_remark() {
        assert_eq!(
            MatchDocNo::from_remark("销售日期:2026-07-28 单据号:收款ZEXQ000062").as_deref(),
            Some("260728ZEXQ000062")
        );
        assert_eq!(
            MatchDocNo::from_remark("购机日期：2026-8-9 单据收款号：ZHLT0000524").as_deref(),
            Some("260809ZHLT000524")
        );
        assert_eq!(
            MatchDocNo::from_remark("销售日期2026-26-29 单据号ZEXQ000062").as_deref(),
            Some("260629ZEXQ000062")
        );
        assert_eq!(
            MatchDocNo::from_remark("销售日期：26.02.16、单据号：ZEXQ000062").as_deref(),
            Some("260216ZEXQ000062")
        );
        assert_eq!(
            MatchDocNo::from_remark("销售日期：2026-2月21日 单据号：ZEXQ000062").as_deref(),
            Some("260221ZEXQ000062")
        );
        // “开具购物发票日期”不得被误当作销售/购机日期使用；缺少销售/购机日期时留空。
        assert_eq!(
            MatchDocNo::from_remark("开具购物发票日期：2026-07-28 单据号：ZEXQ000062"),
            None
        );
        assert_eq!(MatchDocNo::from_remark(""), None);
        assert_eq!(MatchDocNo::from_remark("红冲"), None);
        assert_eq!(
            MatchDocNo::from_remark("销售日期：2026-07-28 购机日期：2026-07-29 单据号：ZEXQ000062"),
            None
        );
        assert_eq!(
            MatchDocNo::from_remark("销售日期：2026-07-28 单据号：ZEXQ000062 单据号：ZEXQ000063"),
            None
        );
        assert_eq!(
            MatchDocNo::from_remark("销售日期：2026-07-28 销售日期：2026-07-28 单据号：ZEXQ000062")
                .as_deref(),
            Some("260728ZEXQ000062")
        );
    }
}
