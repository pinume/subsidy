use super::data::SheetData;
use super::reader::{HeaderMap, UploadColumns, cell_to_decimal, cell_to_string};
use calamine::Data;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};

type UploadTotals = (HashMap<String, Decimal>, HashMap<String, i64>);

pub type SalesMatrix = HashMap<(String, String), HashMap<String, (Decimal, i64)>>;

pub const STD_CATEGORIES: [(&str, &[&str]); 7] = [
    (
        "厨卫",
        &["AO史密斯", "万家乐", "方太", "欧意", "海尔", "美的", "老板"],
    ),
    (
        "洗衣机",
        &["博世", "小鸭", "海尔", "美的", "美菱", "西门子"],
    ),
    ("冰箱", &["博世", "海尔", "美的", "美菱", "西门子"]),
    ("彩电", &["TCL", "创维", "海信", "海尔"]),
    (
        "空调",
        &["TCL", "奥克斯", "格力", "海信", "海尔", "科龙", "美的"],
    ),
    ("小电", &["沁园"]),
    (
        "数码",
        &[
            "IQOO",
            "OPPO",
            "VIVO",
            "一加",
            "作业帮",
            "华为",
            "学而思",
            "小天才",
            "小米",
            "步步高",
            "真我（REALME）",
            "苹果",
            "荣耀",
        ],
    ),
];

pub fn div_pct(num: Decimal, den: Decimal) -> f64 {
    if den.is_zero() {
        0.0
    } else {
        (num / den).to_f64().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MetricRow {
    pub app_amt: Decimal,
    pub app_cnt: i64,
    pub dig_amt: Decimal,
    pub dig_cnt: i64,
}

impl MetricRow {
    pub fn new(app_amt: Decimal, app_cnt: i64, dig_amt: Decimal, dig_cnt: i64) -> Self {
        Self {
            app_amt,
            app_cnt,
            dig_amt,
            dig_cnt,
        }
    }

    pub fn tot_amt(&self) -> Decimal {
        self.app_amt + self.dig_amt
    }

    pub fn tot_cnt(&self) -> i64 {
        self.app_cnt + self.dig_cnt
    }
}

#[derive(Debug, Clone)]
pub struct SummaryMetrics {
    pub occur: MetricRow,
    pub paid: MetricRow,
    pub unpaid: MetricRow,
    pub pass: MetricRow,
    pub wait: MetricRow,
    pub fail: MetricRow,
    pub unup: MetricRow,
}

impl SummaryMetrics {
    pub(crate) fn calculate(
        sales: &SheetData,
        app_up: &SheetData,
        dig_up: &SheetData,
    ) -> Result<Self, String> {
        let sales_h = &sales.header;
        let cat_idx = sales_h.find(&["财务大类"]).ok_or("销售表缺少财务大类")?;
        let sub_idx = sales_h.find(&["补贴额"]).ok_or("销售表缺少补贴额")?;
        let qty_idx = sales_h.find(&["数量"]).ok_or("销售表缺少数量")?;

        let mut app_gen_amt = Decimal::ZERO;
        let mut app_gen_cnt = Decimal::ZERO;
        let mut dig_gen_amt = Decimal::ZERO;
        let mut dig_gen_cnt = Decimal::ZERO;

        for row in &sales[1..] {
            let cat = cell_to_string(&row[cat_idx]);
            let amt = cell_to_decimal(&row[sub_idx]).unwrap_or(Decimal::ZERO);
            let qty = cell_to_decimal(&row[qty_idx]).unwrap_or(Decimal::ZERO);

            if cat == "数码" {
                dig_gen_amt += amt;
                dig_gen_cnt += qty;
            } else {
                app_gen_amt += amt;
                app_gen_cnt += qty;
            }
        }

        let count_uploaded = |rows: &SheetData, label: &str| -> Result<UploadTotals, String> {
            let cols = UploadColumns::from_header(&rows.header, label)?;
            let mut amt_map = HashMap::new();
            let mut cnt_map = HashMap::new();

            for row in &rows[1..] {
                let st = cell_to_string(&row[cols.status]);
                let amt = cell_to_decimal(&row[cols.subsidy]).unwrap_or(Decimal::ZERO);
                *amt_map.entry(st.clone()).or_insert(Decimal::ZERO) += amt;
                *cnt_map.entry(st).or_insert(0) += 1;
            }
            Ok((amt_map, cnt_map))
        };

        let (app_amt, app_cnt) = count_uploaded(app_up, "已上传家电电脑.xlsx")?;
        let (dig_amt, dig_cnt) = count_uploaded(dig_up, "已上传数码.xlsx")?;

        let get_val =
            |map: &HashMap<String, Decimal>, k: &str| *map.get(k).unwrap_or(&Decimal::ZERO);
        let get_cnt = |map: &HashMap<String, i64>, k: &str| *map.get(k).unwrap_or(&0);

        let app_paid_amt = get_val(&app_amt, "已回款");
        let app_paid_cnt = get_cnt(&app_cnt, "已回款");
        let dig_paid_amt = get_val(&dig_amt, "已回款");
        let dig_paid_cnt = get_cnt(&dig_cnt, "已回款");

        let app_unpaid_amt = app_gen_amt - app_paid_amt;
        let app_unpaid_cnt = app_gen_cnt.to_i64().unwrap_or(0) - app_paid_cnt;
        let dig_unpaid_amt = dig_gen_amt - dig_paid_amt;
        let dig_unpaid_cnt = dig_gen_cnt.to_i64().unwrap_or(0) - dig_paid_cnt;

        let app_pass_amt = get_val(&app_amt, "审核通过未回款");
        let app_pass_cnt = get_cnt(&app_cnt, "审核通过未回款");
        let dig_pass_amt = get_val(&dig_amt, "审核通过未回款");
        let dig_pass_cnt = get_cnt(&dig_cnt, "审核通过未回款");

        let app_wait_amt = get_val(&app_amt, "待审核");
        let app_wait_cnt = get_cnt(&app_cnt, "待审核");
        let dig_wait_amt = get_val(&dig_amt, "待审核");
        let dig_wait_cnt = get_cnt(&dig_cnt, "待审核");

        let app_fail_amt = get_val(&app_amt, "审核失败");
        let app_fail_cnt = get_cnt(&app_cnt, "审核失败");
        let dig_fail_amt = get_val(&dig_amt, "审核失败");
        let dig_fail_cnt = get_cnt(&dig_cnt, "审核失败");

        let app_unup_amt = app_unpaid_amt - app_pass_amt - app_wait_amt - app_fail_amt;
        let app_unup_cnt = app_unpaid_cnt - app_pass_cnt - app_wait_cnt - app_fail_cnt;
        let dig_unup_amt = dig_unpaid_amt - dig_pass_amt - dig_wait_amt - dig_fail_amt;
        let dig_unup_cnt = dig_unpaid_cnt - dig_pass_cnt - dig_wait_cnt - dig_fail_cnt;

        Ok(Self {
            occur: MetricRow::new(
                app_gen_amt,
                app_gen_cnt.to_i64().unwrap_or(0),
                dig_gen_amt,
                dig_gen_cnt.to_i64().unwrap_or(0),
            ),
            paid: MetricRow::new(app_paid_amt, app_paid_cnt, dig_paid_amt, dig_paid_cnt),
            unpaid: MetricRow::new(
                app_unpaid_amt,
                app_unpaid_cnt,
                dig_unpaid_amt,
                dig_unpaid_cnt,
            ),
            pass: MetricRow::new(app_pass_amt, app_pass_cnt, dig_pass_amt, dig_pass_cnt),
            wait: MetricRow::new(app_wait_amt, app_wait_cnt, dig_wait_amt, dig_wait_cnt),
            fail: MetricRow::new(app_fail_amt, app_fail_cnt, dig_fail_amt, dig_fail_cnt),
            unup: MetricRow::new(app_unup_amt, app_unup_cnt, dig_unup_amt, dig_unup_cnt),
        })
    }
}

pub fn build_sales_matrix(sales: &SheetData) -> Result<SalesMatrix, String> {
    let h = &sales.header;
    let cat_idx = h.require("财务大类", "销售用券情况统计.xlsx")?;
    let brand_idx = h.require("品牌", "销售用券情况统计.xlsx")?;
    let sub_idx = h.require("补贴额", "销售用券情况统计.xlsx")?;
    let qty_idx = h.require("数量", "销售用券情况统计.xlsx")?;
    let remark_idx = h.require("备注", "销售用券情况统计.xlsx")?;

    let mut matrix: SalesMatrix = HashMap::new();

    for row in &sales[1..] {
        let remark = cell_to_string(&row[remark_idx]);
        if remark == "退货-原单" || remark == "退货-退单" {
            continue;
        }
        let cat = cell_to_string(&row[cat_idx]);
        let brand = cell_to_string(&row[brand_idx]);
        let amt = cell_to_decimal(&row[sub_idx]).unwrap_or(Decimal::ZERO);
        let qty = cell_to_decimal(&row[qty_idx])
            .and_then(|d| d.to_i64())
            .unwrap_or(0);

        let entry = matrix.entry((cat, brand)).or_default();
        let status_entry = entry.entry(remark).or_insert((Decimal::ZERO, 0));
        status_entry.0 += amt;
        status_entry.1 += qty;
    }

    Ok(matrix)
}

pub fn build_invoice_name_map(invoices: &SheetData) -> Result<HashMap<String, String>, String> {
    let inv_h = &invoices.header;
    let inv_no_idx = inv_h
        .find(&["数电发票号码"])
        .ok_or("发票明细缺少数电发票号码")?;
    let inv_name_idx = inv_h
        .find(&["主要商品名称"])
        .ok_or("发票明细缺少主要商品名称")?;
    let mut invoice_name_map = HashMap::new();
    for row in &invoices[1..] {
        let no = cell_to_string(&row[inv_no_idx]);
        let name = cell_to_string(&row[inv_name_idx]);
        if !no.is_empty() {
            invoice_name_map.insert(no, name);
        }
    }
    Ok(invoice_name_map)
}

pub(crate) struct AnomalyColors {
    pub app_pink: HashSet<u32>,
    pub dig_pink: HashSet<u32>,
    pub invoice_yellow: HashSet<u32>,
    pub invoice_pink: HashSet<u32>,
}

pub(crate) struct SummaryRecords {
    pub app_failed: Vec<usize>,
    pub dig_failed: Vec<usize>,
    pub app_refund: Vec<usize>,
    pub dig_refund: Vec<usize>,
    pub invoices: Vec<usize>,
}

impl SummaryRecords {
    // Keep source row indices: Excel and Markdown apply their own presentation ordering.
    pub fn new(
        sources: [&[Vec<Data>]; 5],
        headers: [&HeaderMap; 5],
        colors: &AnomalyColors,
        stores: [&str; 2],
        invoice_names: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let [app, dig, invoices, app_refund, dig_refund] = sources;
        let [app_h, dig_h, _, app_refund_h, dig_refund_h] = headers;
        Ok(Self {
            app_failed: failed_indices(app, app_h, Some(invoice_names))?,
            dig_failed: failed_indices(dig, dig_h, None)?,
            app_refund: refund_indices(app_refund, app_refund_h, &colors.app_pink, stores[0])?,
            dig_refund: refund_indices(dig_refund, dig_refund_h, &colors.dig_pink, stores[1])?,
            invoices: invoices
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(index, _)| {
                    colors.invoice_yellow.contains(&(*index as u32 + 1))
                        && !colors.invoice_pink.contains(&(*index as u32 + 1))
                })
                .map(|(index, _)| index)
                .collect(),
        })
    }
}

pub(crate) fn failed_indices(
    rows: &[Vec<Data>],
    header: &HeaderMap,
    invoice_names: Option<&HashMap<String, String>>,
) -> Result<Vec<usize>, String> {
    let status = header.require("状态", "已上传明细")?;
    let mut indices: Vec<_> = rows
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, row)| cell_to_string(&row[status]) == "审核失败")
        .map(|(index, _)| index)
        .collect();
    if let Some(names) = invoice_names {
        let invoice = header.require("发票号码", "已上传家电电脑.xlsx")?;
        indices.sort_by_key(|&index| {
            let name = names
                .get(&cell_to_string(&rows[index][invoice]))
                .cloned()
                .unwrap_or_default();
            (name.is_empty(), name)
        });
    }
    Ok(indices)
}

fn refund_indices(
    rows: &[Vec<Data>],
    header: &HeaderMap,
    pink: &HashSet<u32>,
    store: &str,
) -> Result<Vec<usize>, String> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let merchant = header.require("核销商编", "回款明细")?;
    Ok(rows
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(index, row)| {
            cell_to_string(&row[merchant]) == store && pink.contains(&(*index as u32 + 1))
        })
        .map(|(index, _)| index)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_keep_row_order_and_distinguish_color_and_merchant_filters() {
        let rows: Vec<Vec<Data>> = [
            ["状态", "发票号码", "核销商编"],
            ["审核失败", "I1", "C"],
            ["已回款", "I2", "C"],
            ["审核失败", "I3", "D"],
            ["", "I4", "C"],
        ]
        .map(|row| row.map(|value| Data::String(value.into())).to_vec())
        .to_vec();
        let header = HeaderMap::from_header_row(&rows[0]);
        let colors = AnomalyColors {
            app_pink: HashSet::from([2, 4, 5]),
            dig_pink: HashSet::from([2, 4, 5]),
            invoice_yellow: HashSet::from([2, 4, 5]),
            invoice_pink: HashSet::from([4]),
        };
        let names = HashMap::from([("I1".into(), "B".into()), ("I3".into(), "A".into())]);
        let records =
            SummaryRecords::new([&rows; 5], [&header; 5], &colors, ["C", "D"], &names).unwrap();
        assert_eq!(
            (
                records.app_failed,
                records.dig_failed,
                records.app_refund,
                records.dig_refund,
                records.invoices
            ),
            (vec![3, 1], vec![1, 3], vec![1, 4], vec![3], vec![1, 4])
        );
    }
}
