//! 浮点往返精度回归测试（仅测试）。
//!
//! 生产链路：清洗层 `Value::Decimal` → `write_table()`（`Decimal::to_f64`，XLSX 数值单元格）
//! → Calamine 读取为 `Data::Float` → `cell_to_decimal()`（`Decimal::from_f64`）
//! → 报表累计。本模块使用真实的 `write_table()` 写出与 Calamine 读回，不手工构造
//! `Data::Float`。测试不依赖任何真实业务数据。
//!
//! 背景：`cell_to_decimal()` 曾使用 `Decimal::from_f64_retain`，保留二进制浮点尾差，
//! 使理论为零的累计值通不过 `is_zero()`，进而破坏零分母比例与零值显示。现已改用
//! `Decimal::from_f64`，这些测试防止回退；诊断信息用 `--nocapture` 查看。

use super::reader::{cell_to_decimal, read_sheet_rows};
use super::summary_data::{CategoryBrandSummary, div_pct};
use crate::io::xlsx_writer::write_table;
use crate::model::{Column, ColumnType, DecimalScale, Row, Table, Value};
use crate::reporting::data::SheetData;
use crate::test_support::unique_temp_path;
use calamine::Data;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::str::FromStr;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn row(values: Vec<Value>) -> Row {
    Row { values, fill: None }
}

fn roundtrip_table(table: &Table, label: &str) -> Vec<Vec<Data>> {
    let dir = unique_temp_path(label);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.xlsx");
    write_table(table, &path).unwrap();
    let rows = read_sheet_rows(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    rows
}

/// 经真实 XLSX 往返后，逐个单元格转换回 `Decimal`。
fn roundtrip_amounts(values: &[Decimal], label: &str) -> Vec<Decimal> {
    let table = Table {
        columns: vec![Column {
            name: "金额",
            ty: ColumnType::Decimal(DecimalScale::Two),
        }],
        rows: values
            .iter()
            .map(|value| row(vec![Value::Decimal(*value)]))
            .collect(),
    };
    let rows = roundtrip_table(&table, label);
    assert!(
        rows[1..].iter().all(|r| matches!(r[0], Data::Float(_))),
        "往返后的金额应为浮点单元格：{rows:?}"
    );
    rows[1..]
        .iter()
        .map(|r| cell_to_decimal(&r[0]).unwrap())
        .collect()
}

/// 理论上正负抵消为零的金额组合（十进制精确和为 0）。
fn cancelling_cases() -> Vec<(&'static str, Vec<Decimal>)> {
    vec![
        ("0.1+0.2-0.3", vec![dec("0.1"), dec("0.2"), dec("-0.3")]),
        ("0.1-0.1（对照）", vec![dec("0.1"), dec("-0.1")]),
        (
            "0.29+0.01-0.30",
            vec![dec("0.29"), dec("0.01"), dec("-0.30")],
        ),
        (
            "299.85+0.15-300.00",
            vec![dec("299.85"), dec("0.15"), dec("-300.00")],
        ),
        ("0.7+0.1-0.8", vec![dec("0.7"), dec("0.1"), dec("-0.8")]),
    ]
}

#[test]
fn roundtrip_cancellation_sums_to_zero() {
    let mut failures = Vec::new();
    for (index, (name, values)) in cancelling_cases().into_iter().enumerate() {
        assert_eq!(values.iter().sum::<Decimal>(), Decimal::ZERO, "{name}");
        let read = roundtrip_amounts(&values, &format!("cancel-{index}"));
        let total: Decimal = read.iter().sum();
        eprintln!(
            "[抵消] {name}: 往返值 {read:?}，累计 {total}，is_zero={}，round_dp(2)={}",
            total.is_zero(),
            total.round_dp(2)
        );
        if !total.is_zero() {
            failures.push(format!("{name}: 累计 {total}"));
        }
    }
    assert!(
        failures.is_empty(),
        "理论为零的累计结果在往返后非零：{failures:#?}"
    );
}

#[test]
fn zero_denominator_after_roundtrip_returns_zero_percent() {
    let numerator = dec("1");
    let mut failures = Vec::new();
    for (index, (name, values)) in cancelling_cases().into_iter().enumerate() {
        let exact: Decimal = values.iter().sum();
        assert_eq!(div_pct(numerator, exact), 0.0, "精确基准：{name}");

        let denominator: Decimal = roundtrip_amounts(&values, &format!("den-{index}"))
            .iter()
            .sum();
        let ratio = div_pct(numerator, denominator);
        eprintln!("[零分母] {name}: 分母 {denominator}，div_pct={ratio}");
        if ratio != 0.0 {
            failures.push(format!("{name}: 分母 {denominator}，比例 {ratio}"));
        }
    }
    assert!(
        failures.is_empty(),
        "理论零分母在往返后未返回 0.0：{failures:#?}"
    );
}

/// 品类品牌汇总：同一品类、品牌、状态下金额与数量理论上同时抵消为零，
/// 报表应显示“-”（条件见 `summary_wb.rs`：`amt.is_zero() && cnt == 0`）。
#[test]
fn zero_amount_and_count_summary_cell_stays_zero_after_roundtrip() {
    let columns = vec![
        Column {
            name: "财务大类",
            ty: ColumnType::Text,
        },
        Column {
            name: "品牌",
            ty: ColumnType::Text,
        },
        Column {
            name: "补贴额",
            ty: ColumnType::Decimal(DecimalScale::Two),
        },
        Column {
            name: "数量",
            ty: ColumnType::Integer,
        },
        Column {
            name: "备注",
            ty: ColumnType::Text,
        },
    ];
    let mut failures = Vec::new();
    for (index, (name, amounts)) in cancelling_cases().into_iter().enumerate() {
        // 数量：前面的正数笔各 +1，最后一笔负数用对应负数量抵消，净数量为 0。
        let positives = amounts.len() as i64 - 1;
        let rows = amounts
            .iter()
            .enumerate()
            .map(|(i, amount)| {
                let quantity = if i == amounts.len() - 1 {
                    -positives
                } else {
                    1
                };
                row(vec![
                    Value::Text("空调".into()),
                    Value::Text("美的".into()),
                    Value::Decimal(*amount),
                    Value::Integer(quantity),
                    Value::Text("已回款".into()),
                ])
            })
            .collect();
        let table = Table {
            columns: columns.clone(),
            rows,
        };
        let sheet = SheetData::new(roundtrip_table(&table, &format!("cell-{index}")));
        let summary = CategoryBrandSummary::from_sales(&sheet).unwrap();
        let (amount, count) = summary.categories()[0].brands()[0]
            .statuses()
            .next()
            .unwrap();
        let shows_dash = amount.is_zero() && count == 0;
        eprintln!("[汇总格] {name}: 金额 {amount}，数量 {count}，显示“-”={shows_dash}");
        if !shows_dash {
            failures.push(format!("{name}: 金额 {amount}，数量 {count}"));
        }
    }
    assert!(
        failures.is_empty(),
        "理论为零的汇总格往返后不再显示“-”：{failures:#?}"
    );
}

// 本测试有意调用被 clippy.toml 禁用的 `from_f64_retain`，用于对照两种转换的差异。
#[allow(clippy::disallowed_methods)]
#[test]
fn from_f64_and_from_f64_retain_differ_on_binary_fractions() {
    for text in ["0.1", "0.29", "299.85"] {
        let value: f64 = text.parse().unwrap();
        let normal = Decimal::from_f64(value).unwrap();
        let retained = Decimal::from_f64_retain(value).unwrap();
        eprintln!("[转换] {text}: from_f64={normal}，from_f64_retain={retained}");
        assert_eq!(normal, dec(text), "from_f64 应规整为输入文本：{text}");
        assert_eq!(
            retained.round_dp(2),
            dec(text).round_dp(2),
            "保留二进制表示仍应在分位一致：{text}"
        );
    }
    // 0.1 的二进制近似不是精确的十进制 0.1（上游文档中的示例值）。
    assert_ne!(
        Decimal::from_f64_retain(0.1).unwrap(),
        Decimal::from_f64(0.1).unwrap()
    );
}

#[test]
fn roundtrip_values_match_to_the_cent_and_report_strict_drift() {
    let originals: Vec<Decimal> = ["0.1", "0.29", "299.85", "-0.07", "12345.67", "0.01"]
        .iter()
        .map(|text| dec(text))
        .collect();
    let read = roundtrip_amounts(&originals, "values");
    let mut strict_drift = Vec::new();
    for (original, back) in originals.iter().zip(&read) {
        assert_eq!(back.round_dp(2), *original, "往返后分位应一致");
        if back != original {
            strict_drift.push(format!("{original} -> {back}"));
        }
    }
    eprintln!("[严格相等差异] {strict_drift:#?}");
}

#[test]
fn accumulated_roundtrip_totals_match_exact_baseline_to_the_cent() {
    let cases: Vec<(&str, Vec<Decimal>, Decimal)> = vec![
        ("10×0.1", vec![dec("0.1"); 10], dec("1.0")),
        ("1000×0.01", vec![dec("0.01"); 1000], dec("10.00")),
        ("5000×0.29", vec![dec("0.29"); 5000], dec("1450.00")),
    ];
    for (index, (name, values, expected)) in cases.into_iter().enumerate() {
        assert_eq!(values.iter().sum::<Decimal>(), expected, "{name} 精确基准");
        let total: Decimal = roundtrip_amounts(&values, &format!("acc-{index}"))
            .iter()
            .sum();
        eprintln!(
            "[累计] {name}: 往返累计 {total}，偏差 {}，round_dp(2)={}",
            total - expected,
            total.round_dp(2)
        );
        assert_eq!(total.round_dp(2), expected.round_dp(2), "{name}");
    }

    // 差额：发生额 - 已回款，理论为零。
    let occurred = roundtrip_amounts(&[dec("0.1"), dec("0.2")], "diff-occur");
    let paid = roundtrip_amounts(&[dec("0.3")], "diff-paid");
    let unpaid: Decimal = occurred.iter().sum::<Decimal>() - paid.iter().sum::<Decimal>();
    eprintln!(
        "[差额] 发生 0.1+0.2 - 回款 0.3 = {unpaid}，round_dp(2)={}",
        unpaid.round_dp(2)
    );
    assert_eq!(unpaid.round_dp(2), Decimal::ZERO);
}

/// 修复后的边界：大金额、负数及超过两位小数的合法输入（有效数字不超过 15 位）
/// 往返后应与原值严格相等，且不被舍到分。
#[test]
fn roundtrip_preserves_large_negative_and_sub_cent_values() {
    let originals: Vec<Decimal> = [
        "9999999999.99",
        "-12345.6789",
        "0.001",
        "1234567.891",
        "-0.07",
        "0",
    ]
    .iter()
    .map(|text| dec(text))
    .collect();
    let read = roundtrip_amounts(&originals, "edge");
    for (original, back) in originals.iter().zip(&read) {
        assert_eq!(back, original, "往返应严格相等：{original} -> {back}");
    }
}
