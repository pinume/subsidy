use calamine::{ExcelDateTime, ExcelDateTimeType};
use chrono::{NaiveDate, NaiveDateTime, Timelike};

/// Excel 日期序列值（1900 日期系统）转换为日期时间，截断到秒。
/// 复用`calamine`自身的纪元换算（含 1900 闰年 bug 修正），不重新实现该算法。
pub fn datetime_from_serial(serial: f64) -> Option<NaiveDateTime> {
    let datetime = ExcelDateTime::new(serial, ExcelDateTimeType::DateTime, false).as_datetime()?;
    datetime.with_nanosecond(0)
}

/// Excel 日期序列值转换为日期（丢弃时间部分）。
pub fn date_from_serial(serial: f64) -> Option<NaiveDate> {
    datetime_from_serial(serial).map(|dt| dt.date())
}

/// 解析`yyyymmdd`（8 位数字文本）为日期。
pub fn parse_yyyymmdd(text: &str) -> Option<NaiveDate> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y%m%d").ok()
}

/// 解析`yyyy-mm-dd`、`yyyy/mm/dd`（可选带`HH:MM:SS`时间部分）文本为日期时间；
/// 无时间部分时以当天零点补齐。
pub fn parse_date_text(text: &str) -> Option<NaiveDateTime> {
    const DATETIME_FORMATS: &[&str] = &["%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M:%S"];
    const DATE_FORMATS: &[&str] = &["%Y-%m-%d", "%Y/%m/%d"];

    let text = text.trim();
    for format in DATETIME_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, format) {
            return Some(dt);
        }
    }
    for format in DATE_FORMATS {
        if let Ok(date) = NaiveDate::parse_from_str(text, format) {
            return Some(date.and_hms_opt(0, 0, 0).unwrap());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_converts_known_date() {
        // 45943 = 2025-10-13（对照 calamine 文档中的示例）。
        let date = date_from_serial(45943.0).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2025, 10, 13).unwrap());
    }

    #[test]
    fn serial_with_time_truncates_to_second() {
        // 45943.5 = 2025-10-13 12:00:00。
        let dt = datetime_from_serial(45943.5).unwrap();
        assert_eq!(dt.and_utc().timestamp_subsec_nanos(), 0);
        assert_eq!(dt.time().hour(), 12);
    }

    #[test]
    fn parses_yyyymmdd() {
        assert_eq!(
            parse_yyyymmdd("20260914"),
            Some(NaiveDate::from_ymd_opt(2026, 9, 14).unwrap())
        );
    }

    #[test]
    fn rejects_invalid_yyyymmdd() {
        assert_eq!(parse_yyyymmdd("2026914"), None);
        assert_eq!(parse_yyyymmdd("20261399"), None);
        assert_eq!(parse_yyyymmdd("abcdefgh"), None);
    }

    #[test]
    fn parses_dashed_and_slashed_date_text() {
        let expected = NaiveDate::from_ymd_opt(2026, 7, 28)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(parse_date_text("2026-07-28"), Some(expected));
        assert_eq!(parse_date_text("2026/07/28"), Some(expected));
    }

    #[test]
    fn parses_date_text_with_time() {
        let expected = NaiveDate::from_ymd_opt(2026, 7, 28)
            .unwrap()
            .and_hms_opt(13, 5, 9)
            .unwrap();
        assert_eq!(parse_date_text("2026-07-28 13:05:09"), Some(expected));
    }

    #[test]
    fn rejects_unrecognized_text() {
        assert_eq!(parse_date_text("2026年7月28日"), None);
    }
}
