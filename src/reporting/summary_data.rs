use super::data::SheetData;
use super::reader::{HeaderMap, UploadColumns, cell_to_decimal, cell_to_string};
use calamine::Data;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{BTreeMap, HashMap, HashSet};

type UploadTotals = HashMap<String, (Decimal, i64)>;

const STD_CATEGORIES: [(&str, &[&str]); 7] = [
    (
        "厨卫",
        &["AO史密斯", "万家乐", "方太", "欧意", "海尔", "美的", "老板"],
    ),
    (
        "洗衣机",
        &["博世", "小鸭", "海尔", "美的", "美菱", "西门子"],
    ),
    ("冰箱", &["博世", "海信", "海尔", "美的", "美菱", "西门子"]),
    ("彩电", &["TCL", "创维", "华为（终端）", "海信", "海尔"]),
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

    fn subtract(self, other: Self) -> Self {
        Self::new(
            self.app_amt - other.app_amt,
            self.app_cnt - other.app_cnt,
            self.dig_amt - other.dig_amt,
            self.dig_cnt - other.dig_cnt,
        )
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
    /// 数值顺序：发生、已回款、综合回款、未回款、通过、待审、失败、未上传。
    pub(crate) fn amount_ratios(&self) -> [(Decimal, f64, Decimal, f64, Decimal, f64); 8] {
        let combined = MetricRow::new(
            self.paid.app_amt + self.pass.app_amt,
            0,
            self.paid.dig_amt + self.pass.dig_amt,
            0,
        );
        let total = self.occur.tot_amt();
        let ratio = |row: &MetricRow, denominator: &MetricRow| {
            (
                row.app_amt,
                div_pct(row.app_amt, denominator.app_amt),
                row.dig_amt,
                div_pct(row.dig_amt, denominator.dig_amt),
                row.tot_amt(),
                div_pct(row.tot_amt(), denominator.tot_amt()),
            )
        };
        [
            // 合计占比也遵循零分母为零的规则。
            (
                self.occur.app_amt,
                div_pct(self.occur.app_amt, total),
                self.occur.dig_amt,
                div_pct(self.occur.dig_amt, total),
                total,
                div_pct(total, total),
            ),
            ratio(&self.paid, &self.occur),
            ratio(&combined, &self.occur),
            ratio(&self.unpaid, &self.occur),
            ratio(&self.pass, &self.unpaid),
            ratio(&self.wait, &self.unpaid),
            ratio(&self.fail, &self.unpaid),
            ratio(&self.unup, &self.unpaid),
        ]
    }

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
            let mut totals = UploadTotals::new();
            for row in &rows[1..] {
                let status = cell_to_string(&row[cols.status]);
                let amount = cell_to_decimal(&row[cols.subsidy]).unwrap_or(Decimal::ZERO);
                let entry = totals.entry(status).or_default();
                entry.0 += amount;
                entry.1 += 1;
            }
            Ok(totals)
        };

        let app = count_uploaded(app_up, "已上传家电电脑.xlsx")?;
        let dig = count_uploaded(dig_up, "已上传数码.xlsx")?;
        let by_status = |status: &str| {
            let (app_amt, app_cnt) = app.get(status).copied().unwrap_or_default();
            let (dig_amt, dig_cnt) = dig.get(status).copied().unwrap_or_default();
            MetricRow::new(app_amt, app_cnt, dig_amt, dig_cnt)
        };
        let occur = MetricRow::new(
            app_gen_amt,
            app_gen_cnt.to_i64().unwrap_or(0),
            dig_gen_amt,
            dig_gen_cnt.to_i64().unwrap_or(0),
        );
        let paid = by_status("已回款");
        let unpaid = occur.subtract(paid);
        let pass = by_status("审核通过未回款");
        let wait = by_status("待审核");
        let fail = by_status("审核失败");
        let unup = unpaid.subtract(pass).subtract(wait).subtract(fail);
        Ok(Self {
            occur,
            paid,
            unpaid,
            pass,
            wait,
            fail,
            unup,
        })
    }
}

pub(crate) const SUMMARY_STATUSES: [&str; 5] =
    ["已回款", "审核通过未回款", "待审核", "审核失败", "未上传"];

pub(crate) struct BrandSummary {
    pub brand: String,
    pub totals: [(Decimal, i64); 5],
}

pub(crate) struct CategorySummary {
    pub category: String,
    pub brands: Vec<BrandSummary>,
}

/// 品类品牌汇总：聚合、业务排序与五状态投影由同一 module 拥有。
/// Excel 格式与大类合并由渲染负责。
pub(crate) struct CategoryBrandSummary {
    groups: Vec<CategorySummary>,
}

impl CategoryBrandSummary {
    pub fn from_sales(sales: &SheetData) -> Result<Self, String> {
        let h = &sales.header;
        let cat_idx = h.require("财务大类", "销售用券情况统计.xlsx")?;
        let brand_idx = h.require("品牌", "销售用券情况统计.xlsx")?;
        let sub_idx = h.require("补贴额", "销售用券情况统计.xlsx")?;
        let qty_idx = h.require("数量", "销售用券情况统计.xlsx")?;
        let remark_idx = h.require("备注", "销售用券情况统计.xlsx")?;

        let mut categories: BTreeMap<String, BTreeMap<String, [(Decimal, i64); 5]>> =
            BTreeMap::new();
        for row in &sales[1..] {
            let remark = cell_to_string(&row[remark_idx]);
            if remark == "退货-原单" || remark == "退货-退单" {
                continue;
            }
            let totals = categories
                .entry(cell_to_string(&row[cat_idx]))
                .or_default()
                .entry(cell_to_string(&row[brand_idx]))
                .or_default();
            // 非展示状态仍保留实际品类品牌组合，但不计入五状态数值。
            if let Some(status) = SUMMARY_STATUSES.iter().position(|value| *value == remark) {
                totals[status].0 += cell_to_decimal(&row[sub_idx]).unwrap_or(Decimal::ZERO);
                totals[status].1 += cell_to_decimal(&row[qty_idx])
                    .and_then(|d| d.to_i64())
                    .unwrap_or(0);
            }
        }

        let mut groups: Vec<_> = categories
            .into_iter()
            .map(|(category, brands)| {
                let standard_brands = STD_CATEGORIES
                    .iter()
                    .find(|(name, _)| *name == category)
                    .map(|(_, brands)| *brands)
                    .unwrap_or_default();
                let mut brands: Vec<_> = brands
                    .into_iter()
                    .map(|(brand, totals)| BrandSummary { brand, totals })
                    .collect();
                // BTreeMap 已按文本升序提供新增品牌；稳定排序将标准品牌移至业务位置。
                brands.sort_by_key(|brand| {
                    standard_brands
                        .iter()
                        .position(|name| *name == brand.brand)
                        .unwrap_or(usize::MAX)
                });
                CategorySummary { category, brands }
            })
            .collect();
        // 标准品类使用业务顺序，新增品类保持 BTreeMap 的文本升序。
        groups.sort_by_key(|group| {
            STD_CATEGORIES
                .iter()
                .position(|(name, _)| *name == group.category)
                .unwrap_or(usize::MAX)
        });
        Ok(Self { groups })
    }

    pub fn groups(&self) -> &[CategorySummary] {
        &self.groups
    }
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
    // Keep source row indices for workbook presentation ordering.
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

    fn sales(rows: &[&[&str]]) -> SheetData {
        SheetData::new(
            std::iter::once(&["财务大类", "品牌", "补贴额", "数量", "备注"][..])
                .chain(rows.iter().copied())
                .map(|row| {
                    row.iter()
                        .map(|value| Data::String((*value).into()))
                        .collect()
                })
                .collect(),
        )
    }

    #[test]
    fn category_brand_summary_orders_actual_combinations_and_projects_five_statuses() {
        let input = sales(&[
            &["Z", "B", "900", "1", "其他"],
            &["冰箱", "海尔", "10.25", "1", "已回款"],
            &["冰箱", "海尔", "-2.25", "-1", "已回款"],
            &["冰箱", "博世", "5", "2", "待审核"],
            &["冰箱", "Z", "4", "1", "未上传"],
            &["冰箱", "A", "3", "1", "审核失败"],
            &["冰箱", "", "6", "1", "审核通过未回款"],
            &["A", "", "7", "1", ""],
            &["", "", "8", "1", "未上传"],
            &["退货品类", "退货品牌", "20", "1", " 退货-原单 "],
            &["退货品类", "退货品牌", "-20", "-1", "退货-退单"],
        ]);
        let summary = CategoryBrandSummary::from_sales(&input).unwrap();
        let names: Vec<_> = summary
            .groups()
            .iter()
            .flat_map(|group| {
                group
                    .brands
                    .iter()
                    .map(move |brand| (group.category.as_str(), brand.brand.as_str()))
            })
            .collect();
        assert_eq!(
            names,
            [
                ("冰箱", "博世"),
                ("冰箱", "海尔"),
                ("冰箱", ""),
                ("冰箱", "A"),
                ("冰箱", "Z"),
                ("", ""),
                ("A", ""),
                ("Z", "B")
            ]
        );
        let zero = (Decimal::ZERO, 0);
        let brands = &summary.groups()[0].brands;
        assert_eq!(
            brands[0].totals,
            [zero, zero, (Decimal::from(5), 2), zero, zero]
        );
        assert_eq!(
            brands[1].totals,
            [(Decimal::from(8), 0), zero, zero, zero, zero]
        );
        assert_eq!(
            brands[2].totals,
            [zero, (Decimal::from(6), 1), zero, zero, zero]
        );
        assert_eq!(
            brands[3].totals,
            [zero, zero, zero, (Decimal::from(3), 1), zero]
        );
        assert_eq!(
            brands[4].totals,
            [zero, zero, zero, zero, (Decimal::from(4), 1)]
        );
        assert_eq!(
            summary.groups()[1].brands[0].totals[4],
            (Decimal::from(8), 1)
        );
        assert_eq!(summary.groups()[2].brands[0].totals, [zero; 5]);
        assert_eq!(summary.groups()[3].brands[0].totals, [zero; 5]);
    }

    #[test]
    fn zero_amount_ratios_are_zero() {
        let zero = MetricRow::default();
        let metrics = SummaryMetrics {
            occur: zero,
            paid: zero,
            unpaid: zero,
            pass: zero,
            wait: zero,
            fail: zero,
            unup: zero,
        };
        for row in metrics.amount_ratios() {
            assert_eq!(
                row,
                (Decimal::ZERO, 0.0, Decimal::ZERO, 0.0, Decimal::ZERO, 0.0,)
            );
        }
    }

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
