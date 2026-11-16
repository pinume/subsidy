use super::data::{CommonInputs, SheetData};
use super::reader::{UploadColumns, cell_to_decimal, cell_to_string};
use super::unique_index::UniqueIndex;
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
pub(crate) struct MetricRow {
    app_amt: Decimal,
    app_cnt: i64,
    dig_amt: Decimal,
    dig_cnt: i64,
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

    fn tot_amt(&self) -> Decimal {
        self.app_amt + self.dig_amt
    }

    /// 按大类归类口径（第3.2节）的（金额，笔数），依次为家电电脑、数码、合计。
    pub fn by_class(&self) -> [(Decimal, i64); 3] {
        [
            (self.app_amt, self.app_cnt),
            (self.dig_amt, self.dig_cnt),
            (self.tot_amt(), self.app_cnt + self.dig_cnt),
        ]
    }
}

/// 汇总表的（总体行，未回款具体情况行），每行为（行名，数值）；两部分之间是提示行。
pub(crate) type SummaryRows<T, const N: usize> = ([(&'static str, T); N], [(&'static str, T); 4]);

#[derive(Debug, Clone)]
pub(crate) struct SummaryMetrics {
    occur: MetricRow,
    paid: MetricRow,
    unpaid: MetricRow,
    pass: MetricRow,
    wait: MetricRow,
    fail: MetricRow,
    unup: MetricRow,
    /// 实时清分UUID与状态都重复、只计一次的上传记录数。
    duplicate_uploads: usize,
}

impl SummaryMetrics {
    /// 表一“金额与数量汇总”的行名与数值：（总体，未回款具体情况）。
    pub(crate) fn rows(&self) -> SummaryRows<&MetricRow, 3> {
        (
            [
                ("国补发生额", &self.occur),
                ("国补已回款额", &self.paid),
                ("未回款", &self.unpaid),
            ],
            [
                ("审核通过未回款", &self.pass),
                ("待审核", &self.wait),
                ("审核失败", &self.fail),
                ("未上传", &self.unup),
            ],
        )
    }

    /// 报表规则第4.1.1节：重复记录只计一次并输出告警。
    pub(crate) fn duplicate_warning(&self) -> Option<String> {
        (self.duplicate_uploads > 0).then(|| {
            format!(
                "已上传明细：{} 条同一实时清分UUID的重复记录未计入，汇总金额和笔数按更新时间最新的状态统计",
                self.duplicate_uploads
            )
        })
    }

    /// 表二“结构与比率分析”的行名与（金额，比率），依次为家电电脑、数码、合计：
    /// （总体，未回款具体情况）。
    pub(crate) fn amount_ratios(&self) -> SummaryRows<[(Decimal, f64); 3], 4> {
        let combined = MetricRow::new(
            self.paid.app_amt + self.pass.app_amt,
            0,
            self.paid.dig_amt + self.pass.dig_amt,
            0,
        );
        let total = self.occur.tot_amt();
        let ratio = |row: &MetricRow, denominator: &MetricRow| {
            [
                (row.app_amt, div_pct(row.app_amt, denominator.app_amt)),
                (row.dig_amt, div_pct(row.dig_amt, denominator.dig_amt)),
                (row.tot_amt(), div_pct(row.tot_amt(), denominator.tot_amt())),
            ]
        };
        let overall = [
            // 合计占比也遵循零分母为零的规则。
            (
                "国补发生额及占比",
                [
                    (self.occur.app_amt, div_pct(self.occur.app_amt, total)),
                    (self.occur.dig_amt, div_pct(self.occur.dig_amt, total)),
                    (total, div_pct(total, total)),
                ],
            ),
            ("国补回款额及回款率", ratio(&self.paid, &self.occur)),
            (
                "回款+审核通过未回款及综合回款率",
                ratio(&combined, &self.occur),
            ),
            ("未回款额", ratio(&self.unpaid, &self.occur)),
        ];
        let unpaid_detail = [
            ("审核通过未回款", ratio(&self.pass, &self.unpaid)),
            ("待审核", ratio(&self.wait, &self.unpaid)),
            ("审核失败", ratio(&self.fail, &self.unpaid)),
            ("未上传", ratio(&self.unup, &self.unpaid)),
        ];
        (overall, unpaid_detail)
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

        let mut duplicate_uploads = 0;
        let mut count_uploaded = |rows: &SheetData, label: &str| -> Result<UploadTotals, String> {
            let cols = UploadColumns::from_header(&rows.header, label)?;
            let (current, duplicates) = current_uploads(rows, label)?;
            duplicate_uploads += duplicates;
            let mut totals = UploadTotals::new();
            for row in current {
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
            duplicate_uploads,
        })
    }
}

/// 报表规则第4.1.1节：金额和笔数不能因重复导出翻倍。同一`实时清分UUID`按`SheetData::latest_rows`
/// 只取更新时间最新的记录；最新时间相同时每个状态各留一条。UUID 为空无法判断重复，全部保留。
/// 返回保留的行（源顺序）与去掉的行数。不用 UniqueIndex：它把状态不同的重复判为冲突并全部丢弃。
pub(crate) fn current_uploads<'a>(
    rows: &'a SheetData,
    label: &str,
) -> Result<(Vec<&'a Vec<Data>>, usize), String> {
    let uuid_idx = rows.header.require("实时清分UUID", label)?;
    let updated_idx = rows.header.require("更新时间", label)?;
    let status_idx = rows.header.require("状态", label)?;
    let cell =
        |row: &'a Vec<Data>, idx: usize| cell_to_string(row.get(idx).unwrap_or(&Data::Empty));
    let mut seen = HashSet::new();
    let current: Vec<_> = rows
        .latest_rows(uuid_idx, updated_idx)
        .into_iter()
        .filter(|row| {
            let uuid = cell(row, uuid_idx);
            uuid.is_empty() || seen.insert((uuid, cell(row, status_idx)))
        })
        .collect();
    let duplicates = rows.len() - 1 - current.len();
    Ok((current, duplicates))
}

pub(crate) const SUMMARY_STATUSES: [&str; 5] =
    ["已回款", "审核通过未回款", "待审核", "审核失败", "未上传"];

pub(crate) struct BrandSummary {
    brand: String,
    totals: [(Decimal, i64); 5],
}

impl BrandSummary {
    pub fn brand(&self) -> &str {
        &self.brand
    }

    /// 按 `SUMMARY_STATUSES` 顺序返回（金额，数量）。
    pub fn statuses(&self) -> impl Iterator<Item = (Decimal, i64)> + '_ {
        self.totals.iter().copied()
    }
}

pub(crate) struct CategorySummary {
    category: String,
    brands: Vec<BrandSummary>,
}

impl CategorySummary {
    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn brands(&self) -> &[BrandSummary] {
        &self.brands
    }
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

    pub fn categories(&self) -> &[CategorySummary] {
        &self.groups
    }
}

/// 报表规则第4.6节：固定品牌组（表头、清洗后品牌、财务大类限制），按展示顺序排列。
const OCCURRENCE_GROUPS: [(&str, &str, Option<&str>); 13] = [
    ("海尔系", "海尔", None),
    ("美的系", "美的", None),
    ("海信系", "海信", None),
    ("TCL系", "TCL", None),
    ("博世系", "博世", None),
    ("西门子系", "西门子", None),
    ("老板", "老板", None),
    ("方太", "方太", None),
    // 清洗后的史密斯品牌名为 AO史密斯。
    ("史密斯", "AO史密斯", None),
    ("格力空调", "格力", Some("空调")),
    ("创维电视", "创维", Some("彩电")),
    ("华为（数码+新业务）", "华为", Some("数码")),
    ("小米（新业务）", "小米", Some("数码")),
];

pub(crate) const OCCURRENCE_METRICS: [&str; 3] = ["使用金额", "已回款", "未回款"];

/// 国补发生情况：固定品牌组的全量累计使用金额与已回款金额（含退货正负）。
pub(crate) struct BrandOccurrence {
    totals: [(Decimal, Decimal); OCCURRENCE_GROUPS.len()],
}

impl BrandOccurrence {
    pub fn from_sales(sales: &SheetData) -> Result<Self, String> {
        let h = &sales.header;
        let cat_idx = h.require("财务大类", "销售用券情况统计.xlsx")?;
        let brand_idx = h.require("品牌", "销售用券情况统计.xlsx")?;
        let sub_idx = h.require("补贴额", "销售用券情况统计.xlsx")?;
        let remark_idx = h.require("备注", "销售用券情况统计.xlsx")?;
        let mut totals = [(Decimal::ZERO, Decimal::ZERO); OCCURRENCE_GROUPS.len()];
        for row in &sales[1..] {
            let (cat, brand) = (
                cell_to_string(&row[cat_idx]),
                cell_to_string(&row[brand_idx]),
            );
            let Some(group) = OCCURRENCE_GROUPS
                .iter()
                .position(|(_, b, c)| *b == brand && c.is_none_or(|c| c == cat))
            else {
                continue;
            };
            let amount = cell_to_decimal(&row[sub_idx]).unwrap_or(Decimal::ZERO);
            totals[group].0 += amount;
            if cell_to_string(&row[remark_idx]) == "已回款" {
                totals[group].1 += amount;
            }
        }
        Ok(Self { totals })
    }

    /// 按展示顺序返回（表头，[使用金额，已回款，未回款]）。
    pub fn brand_groups(&self) -> impl Iterator<Item = (&'static str, [Decimal; 3])> + '_ {
        OCCURRENCE_GROUPS
            .iter()
            .zip(&self.totals)
            .map(|((label, _, _), &(used, paid))| (*label, [used, paid, used - paid]))
    }
}

pub(crate) fn build_invoice_name_index(
    invoices: &SheetData,
) -> Result<UniqueIndex<String>, String> {
    let inv_h = &invoices.header;
    let inv_no_idx = inv_h
        .find(&["数电发票号码"])
        .ok_or("发票明细缺少数电发票号码")?;
    let inv_name_idx = inv_h
        .find(&["主要商品名称"])
        .ok_or("发票明细缺少主要商品名称")?;
    let mut index = UniqueIndex::default();
    for row in &invoices[1..] {
        let no = cell_to_string(&row[inv_no_idx]);
        let name = cell_to_string(&row[inv_name_idx]);
        if !no.is_empty() {
            index.insert(no, name);
        }
    }
    Ok(index)
}

pub(crate) struct AnomalyColors {
    pub app_pink: HashSet<u32>,
    pub dig_pink: HashSet<u32>,
    pub invoice_yellow: HashSet<u32>,
    pub invoice_pink: HashSet<u32>,
}

/// 已选列、已排序的明细区块（报表规则第4.3–4.5节）：渲染只按表头写列并逐行写入，
/// 取哪些行、哪些列及缺列报错都由提取负责。
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct DetailSection {
    headers: Vec<String>,
    rows: Vec<Vec<Data>>,
    /// 区块内值只出现一次时需标黄的列（异常回款的交易参考号）。
    unique_highlight: Option<u16>,
}

const APPLIANCE_UPLOAD: &str = "已上传家电电脑.xlsx";
const DIGITAL_UPLOAD: &str = "已上传数码.xlsx";

/// 审核失败源行及其附加列（商品名称或 IMEI）。
type FailedRows<'a> = Vec<(&'a Vec<Data>, Vec<Data>)>;

const FAILED_FIELDS: [&str; 7] = [
    "交易日期",
    "检索参考号",
    "描述",
    "发票号码",
    "发票金额",
    "购买方名称",
    "S/N码",
];

impl DetailSection {
    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    pub fn rows(&self) -> &[Vec<Data>] {
        &self.rows
    }

    pub fn unique_highlight_col(&self) -> Option<u16> {
        self.unique_highlight
    }

    /// 前 8 列为固定字段，后接各区块附加列；状态列固定写“审核失败”。
    fn failed(
        rows: FailedRows<'_>,
        source: &SheetData,
        label: &str,
        extra_headers: &[&str],
    ) -> Result<Self, String> {
        let mut headers: Vec<String> = FAILED_FIELDS.iter().map(|f| f.to_string()).collect();
        headers.insert(2, "状态".into());
        headers.extend(extra_headers.iter().map(|h| h.to_string()));
        // 没有失败记录时不要求固定字段存在。
        let columns = if rows.is_empty() {
            Vec::new()
        } else {
            source.select(FAILED_FIELDS, label)?
        };
        let rows = rows
            .into_iter()
            .map(|(row, extras)| {
                let mut values: Vec<Data> = columns
                    .iter()
                    .map(|column| column.cell(row).clone())
                    .collect();
                values.insert(2, Data::String("审核失败".into()));
                values.extend(extras);
                values
            })
            .collect();
        Ok(Self {
            headers,
            rows,
            unique_highlight: None,
        })
    }

    /// 家电电脑审核失败行：按发票号码关联商品名称，商品名称升序，空值在后。
    fn appliance_failed_rows<'a>(
        app_up: &'a SheetData,
        invoice_names: &UniqueIndex<String>,
    ) -> Result<FailedRows<'a>, String> {
        let status_idx = app_up.header.require("状态", "已上传明细")?;
        let inv_idx = app_up.header.require("发票号码", APPLIANCE_UPLOAD)?;
        let mut rows: FailedRows<'a> = current_uploads(app_up, APPLIANCE_UPLOAD)?
            .0
            .into_iter()
            .filter(|row| cell_to_string(&row[status_idx]) == "审核失败")
            .map(|row| {
                let name = invoice_names
                    .get(&cell_to_string(&row[inv_idx]))
                    .cloned()
                    .unwrap_or_default();
                (row, vec![Data::String(name)])
            })
            .collect();
        rows.sort_by_key(|(_, extras)| {
            let name = cell_to_string(&extras[0]);
            (name.is_empty(), name)
        });
        Ok(rows)
    }

    /// 数码审核失败行：保持源顺序，附 IMEI1、IMEI2。
    fn digital_failed_rows(dig_up: &SheetData) -> Result<FailedRows<'_>, String> {
        let status_idx = dig_up.header.require("状态", "已上传明细")?;
        let imei1_idx = dig_up.header.require("IMEI1", DIGITAL_UPLOAD)?;
        let imei2_idx = dig_up.header.require("IMEI2", DIGITAL_UPLOAD)?;
        Ok(current_uploads(dig_up, DIGITAL_UPLOAD)?
            .0
            .into_iter()
            .filter(|row| cell_to_string(&row[status_idx]) == "审核失败")
            .map(|row| {
                let imei = [imei1_idx, imei2_idx]
                    .map(|idx| Data::String(cell_to_string(&row[idx])))
                    .to_vec();
                (row, imei)
            })
            .collect())
    }

    #[cfg(test)]
    pub(crate) fn failed_appliance(
        app_up: &SheetData,
        invoice_names: &UniqueIndex<String>,
    ) -> Result<Self, String> {
        let rows = Self::appliance_failed_rows(app_up, invoice_names)?;
        Self::failed(rows, app_up, APPLIANCE_UPLOAD, &["商品名称"])
    }

    #[cfg(test)]
    pub(crate) fn failed_digital(dig_up: &SheetData) -> Result<Self, String> {
        let rows = Self::digital_failed_rows(dig_up)?;
        Self::failed(rows, dig_up, DIGITAL_UPLOAD, &["IMEI1", "IMEI2"])
    }

    /// 源表前 `width` 列原样输出（表头不足时补空）。
    fn leading_columns(source: &SheetData, width: usize, rows: Vec<&Vec<Data>>) -> Self {
        let take = |row: &Vec<Data>| -> Vec<Data> {
            (0..width)
                .map(|col| row.get(col).cloned().unwrap_or(Data::Empty))
                .collect()
        };
        Self {
            headers: take(&source[0]).iter().map(cell_to_string).collect(),
            rows: rows.into_iter().map(take).collect(),
            unique_highlight: None,
        }
    }

    /// 异常回款：本店核销商编且粉色填充；重复参考号在前、不重复在后、空值最后，组内升序。
    /// 不重复的交易参考号标黄。
    fn refund(
        refund: &SheetData,
        pink: &HashSet<u32>,
        store: &str,
        file_label: &str,
    ) -> Result<Self, String> {
        let merchant_idx = refund.header.require("核销商编", "回款明细")?;
        let ref_idx = refund
            .header
            .find(&["交易参考号"])
            .ok_or_else(|| format!("{file_label} 缺少交易参考号"))?;
        let mut rows: Vec<&Vec<Data>> = refund
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(index, row)| {
                cell_to_string(&row[merchant_idx]) == store && pink.contains(&(*index as u32 + 1))
            })
            .map(|(_, row)| row)
            .collect();
        let reference_counts = rows.iter().fold(HashMap::new(), |mut counts, row| {
            *counts.entry(cell_to_string(&row[ref_idx])).or_insert(0) += 1;
            counts
        });
        rows.sort_by_key(|row| {
            let reference = cell_to_string(&row[ref_idx]);
            (
                reference.is_empty(),
                reference_counts[&reference] == 1,
                reference,
            )
        });
        Ok(Self {
            unique_highlight: Some(ref_idx as u16),
            ..Self::leading_columns(refund, 24, rows)
        })
    }

    /// 异常发票行：黄色且非粉色填充，按匹配单据号升序、空值在后。
    fn invoice_rows<'a>(
        invoices: &'a SheetData,
        colors: &AnomalyColors,
    ) -> Result<Vec<&'a Vec<Data>>, String> {
        let doc_no_idx = invoices
            .header
            .find(&["匹配单据号"])
            .ok_or("发票明细缺少匹配单据号")?;
        let mut rows: Vec<&Vec<Data>> = invoices
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(index, _)| {
                let row_num = *index as u32 + 1;
                colors.invoice_yellow.contains(&row_num) && !colors.invoice_pink.contains(&row_num)
            })
            .map(|(_, row)| row)
            .collect();
        rows.sort_by_key(|row| {
            let doc_no = cell_to_string(&row[doc_no_idx]);
            (doc_no.is_empty(), doc_no)
        });
        Ok(rows)
    }

    /// 异常发票输出前 8 列。
    fn invoices(invoices: &SheetData, rows: Vec<&Vec<Data>>) -> Result<Self, String> {
        if invoices[0].len() < 8 {
            return Err("发票明细.xlsx: 表头不足 8 列".to_string());
        }
        Ok(Self::leading_columns(invoices, 8, rows))
    }
}

/// 汇总报表第3–5张表的明细区块。
pub(crate) struct AnomalyDatasets {
    failed: [DetailSection; 2],
    refunds: [DetailSection; 2],
    invoices: DetailSection,
    warning: Option<String>,
}

impl AnomalyDatasets {
    /// 先取行、后选列：多处输入同时出错时，先报的错与按表渲染时一致。
    pub fn extract(inputs: &CommonInputs, colors: &AnomalyColors) -> Result<Self, String> {
        let invoice_names = build_invoice_name_index(&inputs.invoices)?;
        let app_failed = DetailSection::appliance_failed_rows(&inputs.app_upload, &invoice_names)?;
        let dig_failed = DetailSection::digital_failed_rows(&inputs.dig_upload)?;
        let refunds = [
            DetailSection::refund(
                &inputs.app_refund,
                &colors.app_pink,
                &inputs.app_store_code,
                "回款明细家电电脑.xlsx",
            )?,
            DetailSection::refund(
                &inputs.dig_refund,
                &colors.dig_pink,
                &inputs.dig_store_code,
                "回款明细数码.xlsx",
            )?,
        ];
        let invoice_rows = DetailSection::invoice_rows(&inputs.invoices, colors)?;
        Ok(Self {
            failed: [
                DetailSection::failed(
                    app_failed,
                    &inputs.app_upload,
                    APPLIANCE_UPLOAD,
                    &["商品名称"],
                )?,
                DetailSection::failed(
                    dig_failed,
                    &inputs.dig_upload,
                    DIGITAL_UPLOAD,
                    &["IMEI1", "IMEI2"],
                )?,
            ],
            refunds,
            invoices: DetailSection::invoices(&inputs.invoices, invoice_rows)?,
            warning: invoice_names.warning("发票商品名称索引"),
        })
    }

    /// 审核失败：家电电脑、数码。
    pub fn failed(&self) -> &[DetailSection; 2] {
        &self.failed
    }

    /// 异常回款：家电电脑、数码。
    pub fn refunds(&self) -> &[DetailSection; 2] {
        &self.refunds
    }

    pub fn invoices(&self) -> &DetailSection {
        &self.invoices
    }

    /// 发票商品名称索引的冲突告警。
    pub fn warning(&self) -> Option<String> {
        self.warning.clone()
    }
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

    /// 报表规则第4.6节：品牌归组、品类限制、已回款口径、退货相抵、无数据组与负未回款。
    #[test]
    fn brand_occurrence_groups_fixed_brands_and_nets_returns() {
        let input = sales(&[
            &["冰箱", "海尔", "100.50", "1", "已回款"],
            &["洗衣机", "海尔", "20", "1", "待审核"],
            &["空调", "格力", "30", "1", "已回款"],
            &["冰箱", "格力", "999", "1", "已回款"],
            &["彩电", "创维", "50", "1", "退货-原单"],
            &["彩电", "创维", "-50", "-1", "退货-退单"],
            &["数码", "华为", "10", "1", "已回款"],
            &["彩电", "华为（终端）", "999", "1", "已回款"],
            &["厨卫", "AO史密斯", "40", "1", "已回款"],
            &["厨卫", "AO史密斯", "-40", "-1", "未上传"],
            &["数码", "OPPO", "999", "1", "已回款"],
        ]);
        let groups: Vec<_> = BrandOccurrence::from_sales(&input)
            .unwrap()
            .brand_groups()
            .collect();
        assert_eq!(groups.len(), 13);
        let d = |cents: i64| Decimal::new(cents, 2);
        let expect = |label: &str, used: i64, paid: i64, unpaid: i64| {
            let (_, values) = groups.iter().find(|(name, _)| *name == label).unwrap();
            assert_eq!(*values, [d(used), d(paid), d(unpaid)], "{label}");
        };
        expect("海尔系", 12050, 10050, 2000);
        expect("格力空调", 3000, 3000, 0);
        expect("创维电视", 0, 0, 0);
        expect("华为（数码+新业务）", 1000, 1000, 0);
        expect("史密斯", 0, 4000, -4000);
        expect("小米（新业务）", 0, 0, 0);
        for (label, [used, paid, unpaid]) in &groups {
            assert_eq!(*used, paid + unpaid, "{label}");
        }
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
            .categories()
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
        let brands = &summary.categories()[0].brands;
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
            summary.categories()[1].brands[0].totals[4],
            (Decimal::from(8), 1)
        );
        assert_eq!(summary.categories()[2].brands[0].totals, [zero; 5]);
        assert_eq!(summary.categories()[3].brands[0].totals, [zero; 5]);
    }

    #[test]
    fn repeated_exports_count_once_per_uuid_and_status() {
        let text = |value: &str| Data::String(value.into());
        let sales = SheetData::new(vec![vec![text("财务大类"), text("补贴额"), text("数量")]]);
        let header = [
            "实时清分UUID",
            "状态",
            "补贴金额",
            "检索参考号",
            "发票号码",
            "更新时间",
        ];
        let at = |time: &str| Data::DateTimeIso(format!("2026-11-01T{time}"));
        let timed_row = |uuid: &str, status: &str, updated_at: Data| {
            vec![
                text(uuid),
                text(status),
                Data::Int(10),
                text(""),
                text(""),
                updated_at,
            ]
        };
        let row = |uuid: &str, status: &str| timed_row(uuid, status, Data::Empty);
        let uploads = SheetData::new(vec![
            header.iter().map(|name| text(name)).collect(),
            row("A", "待审核"),
            row("A", "待审核"),
            row("A", "审核失败"),
            row("", "待审核"),
            row("", "待审核"),
            // 重新提交后以`更新时间`最新的状态为准。
            timed_row("B", "待审核", at("09:00:00")),
            timed_row("B", "审核失败", at("10:00:00")),
        ]);
        let empty = SheetData::new(vec![header.iter().map(|name| text(name)).collect()]);
        let metrics = SummaryMetrics::calculate(&sales, &uploads, &empty).unwrap();
        // 空 UUID 无法判断重复，全部保留。
        assert_eq!(metrics.wait.app_cnt, 3);
        assert_eq!(metrics.wait.app_amt, Decimal::from(30));
        assert_eq!(metrics.fail.app_cnt, 2);
        assert_eq!(metrics.duplicate_uploads, 2);
        assert_eq!(
            metrics.duplicate_warning().as_deref(),
            Some(
                "已上传明细：2 条同一实时清分UUID的重复记录未计入，汇总金额和笔数按更新时间最新的状态统计"
            )
        );
    }

    #[test]
    fn failed_records_follow_the_same_uuid_dedup_as_metrics() {
        let text = |value: &str| Data::String(value.into());
        let header = [
            "状态",
            "发票号码",
            "实时清分UUID",
            "更新时间",
            "IMEI1",
            "IMEI2",
            "交易日期",
            "检索参考号",
            "描述",
            "发票金额",
            "购买方名称",
            "S/N码",
        ];
        let row = |uuid: &str, status: &str, time: &str| {
            let mut row = vec![
                text(status),
                text(""),
                text(uuid),
                Data::DateTimeIso(format!("2026-11-01T{time}")),
            ];
            row.resize(header.len(), text(""));
            row
        };
        let uploads = SheetData::new(vec![
            header.iter().map(|name| text(name)).collect(),
            row("X", "审核失败", "09:00:00"),
            row("X", "审核失败", "09:00:00"),
            // 失败后重新提交并通过：不再列入失败明细。
            row("Y", "审核失败", "09:00:00"),
            row("Y", "待审核", "10:00:00"),
        ]);
        let names = UniqueIndex::default();
        assert_eq!(
            DetailSection::failed_appliance(&uploads, &names)
                .unwrap()
                .rows()
                .len(),
            1
        );
        assert_eq!(
            DetailSection::failed_digital(&uploads)
                .unwrap()
                .rows()
                .len(),
            1
        );
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
            duplicate_uploads: 0,
        };
        assert_eq!(metrics.duplicate_warning(), None);
        let (overall, unpaid_detail) = metrics.amount_ratios();
        for (_, values) in overall.into_iter().chain(unpaid_detail) {
            assert_eq!(values, [(Decimal::ZERO, 0.0); 3]);
        }
    }

    #[test]
    fn anomaly_datasets_extract_orders_and_groups_sections() {
        // 按表头补齐空单元格。
        let sheet = |rows: &[&[&str]]| {
            let width = rows[0].len();
            SheetData::new(
                rows.iter()
                    .map(|row| {
                        let mut row: Vec<Data> =
                            row.iter().map(|v| Data::String((*v).into())).collect();
                        row.resize(width, Data::String(String::new()));
                        row
                    })
                    .collect(),
            )
        };
        let failed_tail = [
            "实时清分UUID",
            "更新时间",
            "交易日期",
            "检索参考号",
            "描述",
            "发票金额",
            "购买方名称",
            "S/N码",
        ];
        let app_header: Vec<&str> = ["状态", "发票号码"]
            .into_iter()
            .chain(failed_tail)
            .collect();
        let dig_header: Vec<&str> = ["状态", "IMEI1", "IMEI2", "发票号码"]
            .into_iter()
            .chain(failed_tail)
            .collect();
        let refund = sheet(&[
            &["核销商编", "交易参考号"],
            &["S1", "R_SINGLE"],
            &["S1", "R_DUP"],
            &["S1", "R_DUP"],
            &["S1", ""],
            &["OTHER", "R_DUP"],
        ]);
        let mut inputs = CommonInputs {
            sales: sheet(&[&["财务大类"]]),
            app_upload: sheet(&[
                &app_header,
                &["审核失败", "INV2"],
                &["已回款", "INV1"],
                &["审核失败", "INV1"],
                &["审核失败", "INV_NONE"],
            ]),
            dig_upload: sheet(&[
                &dig_header,
                &["审核失败", "EM1", "EM2"],
                &["审核通过", "EM3", "EM4"],
                &["审核失败", "EM5", "EM6"],
            ]),
            invoices: sheet(&[
                &[
                    "匹配单据号",
                    "数电发票号码",
                    "主要商品名称",
                    "4",
                    "5",
                    "6",
                    "7",
                    "8",
                ],
                &["DOC2", "INV2", "美的空调"],
                &["DOC1", "INV1", "海尔冰箱"],
                &[""],
                &["DOC3"],
            ]),
            app_refund: refund,
            dig_refund: sheet(&[&["核销商编", "交易参考号"]]),
            app_store_code: "S1".into(),
            dig_store_code: "S1".into(),
        };
        let colors = AnomalyColors {
            app_pink: HashSet::from([2, 3, 4, 5]),
            dig_pink: HashSet::new(),
            invoice_yellow: HashSet::from([2, 3, 4, 5]),
            invoice_pink: HashSet::from([5]), // Row 5 is pink -> excluded from yellow
        };
        let datasets = AnomalyDatasets::extract(&inputs, &colors).unwrap();
        assert_eq!(datasets.warning(), None);
        let column = |section: &DetailSection, col: usize| -> Vec<String> {
            section
                .rows()
                .iter()
                .map(|row| cell_to_string(&row[col]))
                .collect()
        };

        // 1. App failed sorted by product name (INV1 "海尔冰箱" < INV2 "美的空调" < empty)
        let [app, dig] = datasets.failed();
        assert_eq!(app.headers().len(), 9);
        assert_eq!(column(app, 8), ["海尔冰箱", "美的空调", ""]);
        assert_eq!(column(app, 2), ["审核失败"; 3]);

        // 2. Dig failed extracts IMEI1 and IMEI2 in original order
        assert_eq!(&dig.headers()[8..], ["IMEI1", "IMEI2"]);
        assert_eq!(column(dig, 8), ["EM1", "EM5"]);
        assert_eq!(column(dig, 9), ["EM2", "EM6"]);

        // 3. Invoices sorted by doc_no ascending with empty last (DOC1 < DOC2 < empty)
        let invoices = datasets.invoices();
        assert_eq!(invoices.headers().len(), 8);
        assert_eq!(column(invoices, 0), ["DOC1", "DOC2", ""]);

        // 4. Refund grouped: duplicates first (R_DUP, R_DUP), singletons second (R_SINGLE), empty last
        let [app_refund, dig_refund] = datasets.refunds();
        assert_eq!(app_refund.unique_highlight_col(), Some(1));
        assert_eq!(app_refund.headers().len(), 24);
        assert_eq!(column(app_refund, 1), ["R_DUP", "R_DUP", "R_SINGLE", ""]);
        assert!(dig_refund.rows().is_empty());

        // 多处同时出错时，先报的错与改动前一致：回款缺列先于审核失败缺固定列，
        // 匹配单据号先于发票表头列数。
        let error = |inputs: &CommonInputs| AnomalyDatasets::extract(inputs, &colors).err();
        inputs.app_upload = sheet(&[
            &["状态", "发票号码", "实时清分UUID", "更新时间"],
            &["审核失败", "INV1"],
        ]);
        inputs.dig_refund = sheet(&[&["核销商编"]]);
        assert_eq!(
            error(&inputs).as_deref(),
            Some("回款明细数码.xlsx 缺少交易参考号")
        );
        inputs.dig_refund = sheet(&[&["核销商编", "交易参考号"]]);
        inputs.invoices = sheet(&[&["数电发票号码", "主要商品名称"]]);
        assert_eq!(error(&inputs).as_deref(), Some("发票明细缺少匹配单据号"));
    }

    #[test]
    fn build_invoice_name_index_filters_out_conflicting_names_and_warns() {
        use calamine::Data;
        let rows = vec![
            vec![
                Data::String("数电发票号码".into()),
                Data::String("主要商品名称".into()),
            ],
            vec![Data::String("INV1".into()), Data::String("商品A".into())],
            vec![Data::String("INV1".into()), Data::String("商品B".into())],
            vec![Data::String("INV2".into()), Data::String("商品C".into())],
            vec![Data::String("INV2".into()), Data::String("商品C".into())],
        ];
        let invoices = SheetData::new(rows);
        let index = build_invoice_name_index(&invoices).unwrap();
        assert_eq!(index.get("INV1"), None);
        assert_eq!(index.get("INV2"), Some(&"商品C".to_string()));
        assert_eq!(
            index.warning("发票商品名称索引").as_deref(),
            Some("发票商品名称索引：1 个键对应多条内容不同的记录，已按未命中处理（INV1）")
        );
    }
}
