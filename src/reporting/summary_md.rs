use calamine::Data;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::reader::{HeaderMap, cell_to_decimal, cell_to_string};
use super::summary_data::{
    AnomalyColors, MetricRow, STD_CATEGORIES, SalesMatrix, SummaryMetrics, SummaryRecords, div_pct,
};

/// Generate Markdown report: 国补上传情况汇总.md
#[allow(clippy::too_many_arguments)]
pub fn generate_summary_markdown(
    output_path: &Path,
    metrics: &SummaryMetrics,
    sales_matrix: &SalesMatrix,
    app_up: &[Vec<Data>],
    dig_up: &[Vec<Data>],
    invoices: &[Vec<Data>],
    invoice_name_map: &HashMap<String, String>,
    inv_yellow_rows: &HashSet<u32>,
    inv_pink_rows: &HashSet<u32>,
    app_refund_rows: &[Vec<Data>],
    dig_refund_rows: &[Vec<Data>],
    app_pink_rows: &HashSet<u32>,
    dig_pink_rows: &HashSet<u32>,
    app_store_code: &str,
    dig_store_code: &str,
) -> Result<(), String> {
    let sources = [app_up, dig_up, invoices, app_refund_rows, dig_refund_rows];
    let headers =
        sources.map(|rows| HeaderMap::from_header_row(rows.first().map_or(&[], Vec::as_slice)));
    let colors = AnomalyColors {
        app_pink: app_pink_rows.clone(),
        dig_pink: dig_pink_rows.clone(),
        invoice_yellow: inv_yellow_rows.clone(),
        invoice_pink: inv_pink_rows.clone(),
    };
    let records = SummaryRecords::new(
        sources,
        headers.each_ref(),
        &colors,
        [app_store_code, dig_store_code],
        invoice_name_map,
    )?;
    write_summary_markdown(
        output_path,
        metrics,
        sales_matrix,
        app_up,
        dig_up,
        invoices,
        invoice_name_map,
        app_refund_rows,
        dig_refund_rows,
        &records,
        headers.each_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_summary_markdown(
    output_path: &Path,
    metrics: &SummaryMetrics,
    sales_matrix: &SalesMatrix,
    app_up: &[Vec<Data>],
    dig_up: &[Vec<Data>],
    invoices: &[Vec<Data>],
    invoice_name_map: &HashMap<String, String>,
    app_refund_rows: &[Vec<Data>],
    dig_refund_rows: &[Vec<Data>],
    records: &SummaryRecords,
    headers: [&HeaderMap; 5],
) -> Result<(), String> {
    let mut out = String::with_capacity(64 * 1024);

    let m = metrics;
    let tot_gen_amt = m.occur.tot_amt();
    let tot_paid_amt = m.paid.tot_amt();
    let tot_unpaid_amt = m.unpaid.tot_amt();
    let tot_pass_amt = m.pass.tot_amt();
    let tot_wait_amt = m.wait.tot_amt();
    let tot_fail_amt = m.fail.tot_amt();
    let tot_unup_amt = m.unup.tot_amt();
    let tot_comp_amt = tot_paid_amt + tot_pass_amt;

    // ==========================================
    // Report Title & Metadata
    // ==========================================
    out.push_str("# 国补上传情况汇总分析报告\n\n");
    out.push_str("> 📅 **统计基准**：按源数据全量明细自动汇总  \n");
    if div_pct(tot_comp_amt, tot_gen_amt) != 0.0 {
        out.push_str(&format!(
            "> 📊 **综合回款率**：**{:.2}%** 🟢  \n",
            div_pct(tot_comp_amt, tot_gen_amt) * 100.0
        ));
    }
    if !tot_gen_amt.is_zero() {
        out.push_str(&format!(
            "> 🏷️ **国补发生总额**：**{} 万元**  \n",
            format_wan(tot_gen_amt)
        ));
    }
    if !tot_paid_amt.is_zero() {
        out.push_str(&format!(
            "> 💰 **国补已回款额**：**{} 万元**  \n",
            format_wan(tot_paid_amt)
        ));
    }
    if !tot_unpaid_amt.is_zero()
        || [tot_pass_amt, tot_wait_amt, tot_fail_amt, tot_unup_amt]
            .iter()
            .any(|amount| !amount.is_zero())
    {
        let mut details = Vec::new();
        for (name, amount) in [
            ("在途", tot_pass_amt),
            ("待审", tot_wait_amt),
            ("失败", tot_fail_amt),
            ("未传", tot_unup_amt),
        ] {
            if !amount.is_zero() {
                details.push(format!("{} {} 万", name, format_wan(amount)));
            }
        }
        out.push_str(&format!(
            "> ⏳ **待处理未回款**：{}{}  \n",
            if !tot_unpaid_amt.is_zero() {
                format!("**{} 万元**", format_wan(tot_unpaid_amt))
            } else {
                String::new()
            },
            if details.is_empty() {
                String::new()
            } else {
                format!("（{}）", details.join(" / "))
            }
        ));
    }
    out.push_str("> 💡 **金额单位基准**：汇总、比率分析与大类概况统一为 **万元**；品牌分布明细与各明细统一为 **元**。\n\n");
    out.push_str("---\n\n");

    // ==========================================
    // 1. Table 1: Amount & Count Summary
    // ==========================================
    out.push_str("## 一、 金额与数量结构图\n\n```text\n");
    let child_has_value = [&m.pass, &m.wait, &m.fail, &m.unup]
        .iter()
        .any(|row| row_has_value(row));
    if row_has_value(&m.occur)
        || row_has_value(&m.paid)
        || row_has_value(&m.unpaid)
        || child_has_value
    {
        push_metric_node(&mut out, "", "发生额", &m.occur);
        push_metric_details(&mut out, "    · ", &m.occur);
    }
    let visible_top: Vec<_> = [("已回款", &m.paid), ("未回款", &m.unpaid)]
        .into_iter()
        .filter(|(name, row)| row_has_value(row) || (*name == "未回款" && child_has_value))
        .collect();
    for (index, (name, row)) in visible_top.iter().enumerate() {
        let is_last = index + 1 == visible_top.len();
        push_metric_node(&mut out, if is_last { "└── " } else { "├── " }, name, row);
        push_metric_details(&mut out, if is_last { "    · " } else { "│   · " }, row);
        if *name == "未回款" {
            let visible_children: Vec<_> = [
                ("审核通过未回款", &m.pass),
                ("待审核", &m.wait),
                ("审核失败", &m.fail),
                ("未上传", &m.unup),
            ]
            .into_iter()
            .filter(|(_, child)| row_has_value(child))
            .collect();
            for (child_index, (child_name, child)) in visible_children.iter().enumerate() {
                let base = if is_last { "    " } else { "│   " };
                let child_prefix = if child_index + 1 == visible_children.len() {
                    "└── "
                } else {
                    "├── "
                };
                push_metric_node(
                    &mut out,
                    &format!("{}{}", base, child_prefix),
                    child_name,
                    child,
                );
            }
        }
    }
    out.push_str("```\n\n---\n\n");

    // ==========================================
    // 2. Table 2: Structure & Ratio Analysis
    // ==========================================
    out.push_str("## 二、 结构与比率分析图\n\n```text\n");

    let app_comp_amt = m.paid.app_amt + m.pass.app_amt;
    let dig_comp_amt = m.paid.dig_amt + m.pass.dig_amt;

    let t2_rows: [(&str, Decimal, f64, Decimal, f64, Decimal, f64); 8] = [
        (
            "发生额",
            m.occur.app_amt,
            div_pct(m.occur.app_amt, tot_gen_amt),
            m.occur.dig_amt,
            div_pct(m.occur.dig_amt, tot_gen_amt),
            tot_gen_amt,
            1.0,
        ),
        (
            "已回款",
            m.paid.app_amt,
            div_pct(m.paid.app_amt, m.occur.app_amt),
            m.paid.dig_amt,
            div_pct(m.paid.dig_amt, m.occur.dig_amt),
            tot_paid_amt,
            div_pct(tot_paid_amt, tot_gen_amt),
        ),
        (
            "未回款",
            m.unpaid.app_amt,
            div_pct(m.unpaid.app_amt, m.occur.app_amt),
            m.unpaid.dig_amt,
            div_pct(m.unpaid.dig_amt, m.occur.dig_amt),
            tot_unpaid_amt,
            div_pct(tot_unpaid_amt, tot_gen_amt),
        ),
        (
            "审核通过未回款",
            m.pass.app_amt,
            div_pct(m.pass.app_amt, m.unpaid.app_amt),
            m.pass.dig_amt,
            div_pct(m.pass.dig_amt, m.unpaid.dig_amt),
            tot_pass_amt,
            div_pct(tot_pass_amt, tot_unpaid_amt),
        ),
        (
            "待审核",
            m.wait.app_amt,
            div_pct(m.wait.app_amt, m.unpaid.app_amt),
            m.wait.dig_amt,
            div_pct(m.wait.dig_amt, m.unpaid.dig_amt),
            tot_wait_amt,
            div_pct(tot_wait_amt, tot_unpaid_amt),
        ),
        (
            "审核失败",
            m.fail.app_amt,
            div_pct(m.fail.app_amt, m.unpaid.app_amt),
            m.fail.dig_amt,
            div_pct(m.fail.dig_amt, m.unpaid.dig_amt),
            tot_fail_amt,
            div_pct(tot_fail_amt, tot_unpaid_amt),
        ),
        (
            "未上传",
            m.unup.app_amt,
            div_pct(m.unup.app_amt, m.unpaid.app_amt),
            m.unup.dig_amt,
            div_pct(m.unup.dig_amt, m.unpaid.dig_amt),
            tot_unup_amt,
            div_pct(tot_unup_amt, tot_unpaid_amt),
        ),
        (
            "回款+审核通过未回款",
            app_comp_amt,
            div_pct(app_comp_amt, m.occur.app_amt),
            dig_comp_amt,
            div_pct(dig_comp_amt, m.occur.dig_amt),
            tot_comp_amt,
            div_pct(tot_comp_amt, tot_gen_amt),
        ),
    ];

    let visible_t2 = |row: &(&str, Decimal, f64, Decimal, f64, Decimal, f64)| {
        !row.1.is_zero() || !row.3.is_zero() || !row.5.is_zero()
    };
    let ratio_child_has_value = t2_rows[3..7].iter().any(visible_t2);
    if visible_t2(&t2_rows[0])
        || visible_t2(&t2_rows[1])
        || visible_t2(&t2_rows[2])
        || ratio_child_has_value
    {
        push_ratio_node(&mut out, "", &t2_rows[0], "发生占比");
        push_ratio_details(&mut out, "    · ", &t2_rows[0], "发生占比");
    }
    let visible_top: Vec<_> = t2_rows[1..3]
        .iter()
        .filter(|row| visible_t2(row) || (row.0 == "未回款" && ratio_child_has_value))
        .collect();
    for (index, row) in visible_top.iter().enumerate() {
        let is_last = index + 1 == visible_top.len();
        let rate_name = if row.0 == "已回款" {
            "回款率"
        } else {
            "未回款率"
        };
        push_ratio_node(
            &mut out,
            if is_last { "└── " } else { "├── " },
            row,
            rate_name,
        );
        push_ratio_details(
            &mut out,
            if is_last { "    · " } else { "│   · " },
            row,
            rate_name,
        );
        if row.0 == "未回款" {
            let visible_children: Vec<_> = t2_rows[3..7]
                .iter()
                .filter(|child| visible_t2(child))
                .collect();
            for (child_index, child) in visible_children.iter().enumerate() {
                let base = if is_last { "    " } else { "│   " };
                let child_prefix = if child_index + 1 == visible_children.len() {
                    "└── "
                } else {
                    "├── "
                };
                push_ratio_node(
                    &mut out,
                    &format!("{}{}", base, child_prefix),
                    child,
                    "占未回款",
                );
                push_ratio_details(
                    &mut out,
                    &format!(
                        "{}{}",
                        base,
                        if child_index + 1 == visible_children.len() {
                            "    · "
                        } else {
                            "│   · "
                        }
                    ),
                    child,
                    "占未回款",
                );
            }
        }
    }
    let (name, a_amt, a_pct, d_amt, d_pct, t_amt, t_pct) = t2_rows[7];
    if !a_amt.is_zero() || !d_amt.is_zero() || !t_amt.is_zero() {
        out.push_str(&format!(
            "\n派生指标\n{}{}\n",
            name,
            if t_amt.is_zero() {
                String::new()
            } else {
                format!("（{}）", format_amount_ratio(t_amt, t_pct, "综合回款率"))
            }
        ));
        for (kind, amount, pct) in [("家电", a_amt, a_pct), ("数码", d_amt, d_pct)] {
            if !amount.is_zero() {
                out.push_str(&format!(
                    "    · {}：{}\n",
                    kind,
                    format_amount_ratio(amount, pct, "综合回款率")
                ));
            }
        }
    }

    out.push_str("```\n\n---\n\n");

    // ==========================================
    // 3. Category and Brand Summary
    // ==========================================
    out.push_str("## 三、 品类品牌汇总\n\n");

    let matrix = sales_matrix;

    let mut all_cats: Vec<String> = Vec::new();
    for (cat, _) in STD_CATEGORIES {
        if matrix.keys().any(|(c, _)| c == cat) {
            all_cats.push(cat.to_string());
        }
    }
    for (c, _) in matrix.keys() {
        if !all_cats.contains(c) {
            all_cats.push(c.clone());
        }
    }

    // 3.1 Category Overview Table
    // 3.1 Category Overview Table
    type CatSums = (Decimal, Decimal, Decimal, Decimal, Decimal, Decimal, i64);
    let mut cat_sums: HashMap<String, CatSums> = HashMap::new();
    let mut total_occur_amt = Decimal::ZERO;
    let mut total_paid_amt = Decimal::ZERO;
    let mut total_pass_amt = Decimal::ZERO;
    let mut total_wait_amt = Decimal::ZERO;
    let mut total_fail_amt = Decimal::ZERO;
    let mut total_unup_amt = Decimal::ZERO;

    for ((c, _b), status_map) in matrix {
        let entry = cat_sums.entry(c.clone()).or_insert((
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            0,
        ));
        for (st, (amt, cnt)) in status_map {
            entry.0 += *amt;
            entry.6 += *cnt;
            total_occur_amt += *amt;
            match st.as_str() {
                "已回款" => {
                    entry.1 += *amt;
                    total_paid_amt += *amt;
                }
                "审核通过未回款" => {
                    entry.2 += *amt;
                    total_pass_amt += *amt;
                }
                "待审核" => {
                    entry.3 += *amt;
                    total_wait_amt += *amt;
                }
                "审核失败" => {
                    entry.4 += *amt;
                    total_fail_amt += *amt;
                }
                "未上传" => {
                    entry.5 += *amt;
                    total_unup_amt += *amt;
                }
                _ => {}
            }
        }
    }

    out.push_str("### 3.1 各大类经营概况（已排除退货）\n\n");
    out.push_str("> **数据口径**：源自《销售用券情况统计.xlsx》，已严格排除 `退货-原单` 与 `退货-退单`。金额单位：**万元**；综合回款率 =（已回款 + 审核通过未回款）÷ 发生额。\n\n");

    push_category_overview(
        &mut out,
        &CategoryOverview {
            name: "合计",
            occur: total_occur_amt,
            share: 1.0,
            paid: total_paid_amt,
            pass: total_pass_amt,
            wait: total_wait_amt,
            fail: total_fail_amt,
            unup: total_unup_amt,
        },
    );

    for cat in &all_cats {
        if let Some(sums) = cat_sums.get(cat) {
            let cat_occur = sums.0;
            let cat_paid = sums.1;
            let cat_pass = sums.2;
            let cat_wait = sums.3;
            let cat_fail = sums.4;
            let cat_unup = sums.5;
            let share = div_pct(cat_occur, total_occur_amt);
            if !cat_occur.is_zero()
                || !cat_paid.is_zero()
                || !cat_pass.is_zero()
                || !cat_wait.is_zero()
                || !cat_fail.is_zero()
                || !cat_unup.is_zero()
            {
                push_category_overview(
                    &mut out,
                    &CategoryOverview {
                        name: cat,
                        occur: cat_occur,
                        share,
                        paid: cat_paid,
                        pass: cat_pass,
                        wait: cat_wait,
                        fail: cat_fail,
                        unup: cat_unup,
                    },
                );
            }
        }
    }

    // 3.2 Counter Detail List
    out.push_str("### 3.2 品牌分布明细\n\n");
    out.push_str("> **合并口径**：财务大类与品牌合并为“专柜”，数据合并规则为【品牌+财务大类】；其中 AO史密斯、万家乐、方太、欧意、老板、小鸭、奥克斯、格力、科龙、沁园仅保留品牌，数码大类仅保留财务大类【数码】。金额单位：**元**。\n\n");

    let mut counter_matrix: HashMap<String, HashMap<String, (Decimal, i64)>> = HashMap::new();
    for ((cat, brand), status_map) in matrix {
        let counter = map_counter_name(cat, brand);
        let entry = counter_matrix.entry(counter).or_default();
        for (st, (amt, cnt)) in status_map {
            let st_entry = entry.entry(st.clone()).or_insert((Decimal::ZERO, 0));
            st_entry.0 += *amt;
            st_entry.1 += *cnt;
        }
    }

    let statuses = ["已回款", "审核通过未回款", "待审核", "审核失败", "未上传"];
    let mut total_matrix_amt = Decimal::ZERO;
    let mut total_matrix_cnt: i64 = 0;
    let mut status_totals: [(Decimal, i64); 5] = [(Decimal::ZERO, 0); 5];

    for counter_map in counter_matrix.values() {
        for (s_idx, st) in statuses.iter().enumerate() {
            let (amt, cnt) = counter_map.get(*st).copied().unwrap_or((Decimal::ZERO, 0));
            total_matrix_amt += amt;
            total_matrix_cnt += cnt;
            status_totals[s_idx].0 += amt;
            status_totals[s_idx].1 += cnt;
        }
    }

    push_counter_entry(
        &mut out,
        "合计",
        total_matrix_amt,
        total_matrix_cnt,
        &statuses,
        &status_totals,
    );

    let mut seen_counters: std::collections::HashSet<String> = std::collections::HashSet::new();

    for cat in &all_cats {
        let std_brands: Vec<String> = STD_CATEGORIES
            .iter()
            .find(|(c, _)| c == cat)
            .map(|(_, bs)| bs.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();

        let mut cat_counters = Vec::new();
        for b in &std_brands {
            let c_name = map_counter_name(cat, b);
            if counter_matrix.contains_key(&c_name)
                && !cat_counters.contains(&c_name)
                && !seen_counters.contains(&c_name)
            {
                cat_counters.push(c_name.clone());
                seen_counters.insert(c_name);
            }
        }
        let mut new_brands: Vec<String> = matrix
            .keys()
            .filter(|(c, b)| c == cat && !std_brands.contains(b))
            .map(|(_, b)| b.clone())
            .collect();
        new_brands.sort();
        for b in &new_brands {
            let c_name = map_counter_name(cat, b);
            if counter_matrix.contains_key(&c_name)
                && !cat_counters.contains(&c_name)
                && !seen_counters.contains(&c_name)
            {
                cat_counters.push(c_name.clone());
                seen_counters.insert(c_name);
            }
        }

        if cat_counters.is_empty() {
            continue;
        }

        out.push_str(&format!("#### {}\n\n", cat));
        for counter in &cat_counters {
            let empty_map = HashMap::new();
            let counter_map = counter_matrix.get(counter).unwrap_or(&empty_map);

            let mut row_amt = Decimal::ZERO;
            let mut row_cnt: i64 = 0;
            let mut status_values = [(Decimal::ZERO, 0); 5];

            for (s_idx, st) in statuses.iter().enumerate() {
                let (amt, cnt) = counter_map.get(*st).copied().unwrap_or((Decimal::ZERO, 0));
                row_amt += amt;
                row_cnt += cnt;
                status_values[s_idx] = (amt, cnt);
            }

            push_counter_entry(
                &mut out,
                counter,
                row_amt,
                row_cnt,
                &statuses,
                &status_values,
            );
        }
    }

    let mut remaining_counters: Vec<String> = counter_matrix
        .keys()
        .filter(|c| !seen_counters.contains(*c))
        .cloned()
        .collect();
    remaining_counters.sort();
    if !remaining_counters.is_empty() {
        out.push_str("#### 其他\n\n");
        for counter in &remaining_counters {
            let empty_map = HashMap::new();
            let counter_map = counter_matrix.get(counter).unwrap_or(&empty_map);

            let mut row_amt = Decimal::ZERO;
            let mut row_cnt: i64 = 0;
            let mut status_values = [(Decimal::ZERO, 0); 5];

            for (s_idx, st) in statuses.iter().enumerate() {
                let (amt, cnt) = counter_map.get(*st).copied().unwrap_or((Decimal::ZERO, 0));
                row_amt += amt;
                row_cnt += cnt;
                status_values[s_idx] = (amt, cnt);
            }

            push_counter_entry(
                &mut out,
                counter,
                row_amt,
                row_cnt,
                &statuses,
                &status_values,
            );
        }
    }

    out.push_str("---\n\n");

    // ==========================================
    // 4. Audit Failures
    // ==========================================
    out.push_str("## 四、 审核失败明细清单\n\n");

    let app_h = headers[0];
    let app_inv_idx = app_h.require("发票号码", "已上传家电电脑.xlsx")?;
    let app_desc_idx = app_h.require("描述", "已上传家电电脑.xlsx")?;

    let dig_h = headers[1];
    let dig_desc_idx = dig_h.require("描述", "已上传数码.xlsx")?;

    // 4.1 Appliance Failed Detail List
    let app_failed: Vec<_> = records
        .app_failed
        .iter()
        .map(|&index| &app_up[index])
        .collect();

    out.push_str(&format!(
        "### 4.1 家电电脑审核失败明细清单（共 {} 笔）\n\n",
        app_failed.len()
    ));
    out.push_str("> **状态说明**：以下清单全部记录之审核状态均为【审核失败】，包含源数据中的【审核终止】。\n");
    out.push_str("> 排序口径：按关联发票之“商品名称”升序排列，空值置后。\n\n");

    let app_date_idx = app_h.require("交易日期", "已上传家电电脑.xlsx")?;
    let app_ref_idx = app_h.require("检索参考号", "已上传家电电脑.xlsx")?;
    let app_inv_amt_idx = app_h.require("发票金额", "已上传家电电脑.xlsx")?;
    let app_buyer_idx = app_h.require("购买方名称", "已上传家电电脑.xlsx")?;
    let app_sn_idx = app_h
        .find(&["S/N码", "sn码"])
        .ok_or("已上传家电电脑缺少 S/N码")?;

    for (i, row) in app_failed.iter().enumerate() {
        let inv_no = cell_to_string(&row[app_inv_idx]);
        let prod_name = invoice_name_map.get(&inv_no).cloned().unwrap_or_default();
        let inv_amt = cell_to_decimal(&row[app_inv_amt_idx]).unwrap_or(Decimal::ZERO);
        let ref_str = format_code_cell(&cell_to_string(&row[app_ref_idx]));
        let title_extra = if prod_name.is_empty() {
            format!("（{} 元）", format_yuan(inv_amt))
        } else {
            format!(
                "（{} · {} 元）",
                escape_md_cell(&prod_name),
                format_yuan(inv_amt)
            )
        };
        out.push_str(&format!(
            "- **{}. 检索参考号 {}**{}\n```text\n├── ⚠️ 失败原因：{}\n├── 发票号码：{}（购买方：{}）\n└── 交易设备：{} · S/N {}\n```\n\n",
            i + 1,
            ref_str,
            title_extra,
            escape_md_cell(&cell_to_string(&row[app_desc_idx])),
            format_code_cell(&inv_no),
            escape_md_cell(&cell_to_string(&row[app_buyer_idx])),
            format_date_str(&row[app_date_idx]),
            format_code_cell(&cell_to_string(&row[app_sn_idx]))
        ));
    }

    // 4.2 Digital Failed Detail List
    let dig_date_idx = dig_h.require("交易日期", "已上传数码.xlsx")?;
    let dig_ref_idx = dig_h.require("检索参考号", "已上传数码.xlsx")?;
    let dig_inv_idx = dig_h.require("发票号码", "已上传数码.xlsx")?;
    let dig_inv_amt_idx = dig_h.require("发票金额", "已上传数码.xlsx")?;
    let dig_buyer_idx = dig_h.require("购买方名称", "已上传数码.xlsx")?;
    let dig_sn_idx = dig_h
        .find(&["S/N码", "sn码"])
        .ok_or("已上传数码缺少 S/N码")?;
    let dig_imei1_idx = dig_h
        .find(&["IMEI1", "imei1"])
        .ok_or("已上传数码缺少 IMEI1")?;
    let dig_imei2_idx = dig_h
        .find(&["IMEI2", "imei2"])
        .ok_or("已上传数码缺少 IMEI2")?;

    let dig_failed: Vec<_> = records
        .dig_failed
        .iter()
        .map(|&index| &dig_up[index])
        .collect();

    out.push_str(&format!(
        "### 4.2 数码审核失败明细清单（共 {} 笔）\n\n",
        dig_failed.len()
    ));
    out.push_str("> **状态说明**：以下清单全部记录之审核状态均为【审核失败】，包含源数据中的【审核终止】。\n\n");
    for (i, row) in dig_failed.iter().enumerate() {
        let inv_no = cell_to_string(&row[dig_inv_idx]);
        let prod_name = invoice_name_map.get(&inv_no).cloned().unwrap_or_default();
        let inv_amt = cell_to_decimal(&row[dig_inv_amt_idx]).unwrap_or(Decimal::ZERO);
        let ref_str = format_code_cell(&cell_to_string(&row[dig_ref_idx]));
        let title_extra = if prod_name.is_empty() {
            format!("（{} 元）", format_yuan(inv_amt))
        } else {
            format!(
                "（{} · {} 元）",
                escape_md_cell(&prod_name),
                format_yuan(inv_amt)
            )
        };

        let imei1_str = cell_to_string(&row[dig_imei1_idx]);
        let imei2_str = cell_to_string(&row[dig_imei2_idx]);
        let mut device_info = format!(
            "{} · S/N {}",
            format_date_str(&row[dig_date_idx]),
            format_code_cell(&cell_to_string(&row[dig_sn_idx]))
        );
        if !imei1_str.is_empty() && imei1_str != "-" {
            device_info.push_str(&format!(" · IMEI1 {}", format_code_cell(&imei1_str)));
        }
        if !imei2_str.is_empty() && imei2_str != "-" {
            device_info.push_str(&format!(" · IMEI2 {}", format_code_cell(&imei2_str)));
        }

        out.push_str(&format!(
            "- **{}. 检索参考号 {}**{}\n```text\n├── ⚠️ 失败原因：{}\n├── 发票号码：{}（购买方：{}）\n└── 交易设备：{}\n```\n\n",
            i + 1,
            ref_str,
            title_extra,
            escape_md_cell(&cell_to_string(&row[dig_desc_idx])),
            format_code_cell(&inv_no),
            escape_md_cell(&cell_to_string(&row[dig_buyer_idx])),
            device_info
        ));
    }

    out.push_str("---\n\n");

    // ==========================================
    // 5. Abnormal Refunds
    // ==========================================
    out.push_str("## 五、 异常回款明细清单（粉色填充标识）\n\n");
    out.push_str("> **提取规则**：精准筛选属于本店核销商编且在原始回款明细中单元格背景为粉色（#FFC7CE）的数据行。\n");
    out.push_str("> **主体与商编说明**：销售企业统一为【北国商城股份有限公司】；家电电脑核销商编统一为【89813015722APT1】，数码核销商编统一为【89813014812B06R】。\n");
    out.push_str("> **对账规则**：相同交易参考号的冲销对自动合并为同一对账档案，对比展示正向拨付与反向冲销流水及抵扣净额；单笔异常记录单独建档；组内按交易参考号升序排列。\n\n");

    let format_refund_section = |title: &str,
                                 rows: &[Vec<Data>],
                                 indices: &[usize],
                                 h: &HeaderMap|
     -> Result<String, String> {
        let mut s = String::new();
        if rows.is_empty() {
            return Ok(s);
        }
        let ref_idx = h.find(&["交易参考号"]).ok_or("缺少交易参考号")?;
        let batch_idx = h.find(&["拨付批次"]).unwrap_or(0);
        let time_idx = h.find(&["交易完成时间"]).unwrap_or(1);
        let order_idx = h.find(&["商户订单号"]).unwrap_or(3);
        let sub_idx = h.find(&["补贴金额"]).unwrap_or(10);
        let sn_idx = h.find(&["SN码", "sn码"]).unwrap_or(12);
        let prod_idx = h.find(&["商品名称"]).unwrap_or(17);

        let matched_rows: Vec<_> = indices.iter().map(|&index| &rows[index]).collect();

        let mut groups_map: HashMap<String, Vec<&Vec<Data>>> = HashMap::new();
        for row in &matched_rows {
            let reference = cell_to_string(&row[ref_idx]);
            groups_map.entry(reference).or_default().push(row);
        }

        let mut groups: Vec<(String, Vec<&Vec<Data>>)> = groups_map.into_iter().collect();
        groups.sort_by(|(ref_a, rows_a), (ref_b, rows_b)| {
            let is_single_a = rows_a.len() == 1;
            let is_single_b = rows_b.len() == 1;
            is_single_a
                .cmp(&is_single_b)
                .then_with(|| ref_a.is_empty().cmp(&ref_b.is_empty()))
                .then_with(|| ref_a.cmp(ref_b))
        });

        s.push_str(&format!(
            "### {}（共 {} 组档案 · {} 笔流水）\n\n",
            title,
            groups.len(),
            matched_rows.len()
        ));

        for (i, (reference, mut group_rows)) in groups.into_iter().enumerate() {
            let is_duplicate = group_rows.len() > 1;
            let ref_str = format_code_cell(&reference);

            if is_duplicate {
                group_rows.sort_by_key(|r| {
                    let amt = cell_to_decimal(&r[sub_idx]).unwrap_or(Decimal::ZERO);
                    (amt < Decimal::ZERO, amt)
                });

                let net_amt: Decimal = group_rows
                    .iter()
                    .map(|r| cell_to_decimal(&r[sub_idx]).unwrap_or(Decimal::ZERO))
                    .sum();

                let prod_name = cell_to_string(&group_rows[0][prod_idx]);
                let title_prod = if prod_name.is_empty() {
                    String::new()
                } else {
                    format!("{} · ", escape_md_cell(&prod_name))
                };

                let order_no = cell_to_string(&group_rows[0][order_idx]);
                let sn = cell_to_string(&group_rows[0][sn_idx]);

                s.push_str(&format!(
                    "- **{}. 交易参考号 {}**（{}⚠️ 冲销对 · 净额 {} 元）\n```text\n├── 关联档案：商户订单号 {} · S/N {}\n",
                    i + 1,
                    ref_str,
                    title_prod,
                    format_yuan(net_amt),
                    format_code_cell(&order_no),
                    format_code_cell(&sn)
                ));

                for (k, r) in group_rows.iter().enumerate() {
                    let is_last = k + 1 == group_rows.len();
                    let prefix = if is_last { "└── " } else { "├── " };
                    let amt = cell_to_decimal(&r[sub_idx]).unwrap_or(Decimal::ZERO);
                    let label = if amt >= Decimal::ZERO {
                        "拨付流水：+"
                    } else {
                        "冲销流水："
                    };
                    let batch = cell_to_string(&r[batch_idx]);
                    let time = cell_to_string(&r[time_idx]);
                    let time_part = if time.is_empty() {
                        format!("（{}）", escape_md_cell(&batch))
                    } else {
                        format!("（{} · {}）", escape_md_cell(&batch), escape_md_cell(&time))
                    };

                    s.push_str(&format!(
                        "{}{}{} 元{}\n",
                        prefix,
                        label,
                        format_yuan(amt),
                        time_part
                    ));
                }
                s.push_str("```\n\n");
            } else {
                let r = group_rows[0];
                let amt = cell_to_decimal(&r[sub_idx]).unwrap_or(Decimal::ZERO);
                let prod_name = cell_to_string(&r[prod_idx]);
                let title_prod = if prod_name.is_empty() {
                    String::new()
                } else {
                    format!("{} · ", escape_md_cell(&prod_name))
                };

                let order_no = cell_to_string(&r[order_idx]);
                let sn = cell_to_string(&r[sn_idx]);
                let batch = cell_to_string(&r[batch_idx]);
                let time = cell_to_string(&r[time_idx]);
                let time_part = if time.is_empty() {
                    format!("（{}）", escape_md_cell(&batch))
                } else {
                    format!("（{} · {}）", escape_md_cell(&batch), escape_md_cell(&time))
                };

                s.push_str(&format!(
                    "- **{}. 交易参考号 {}**（{}单笔记录 · 补贴 {} 元）\n```text\n├── 关联档案：商户订单号 {} · S/N {}\n└── 拨付流水：{} 元{}\n```\n\n",
                    i + 1,
                    ref_str,
                    title_prod,
                    format_yuan(amt),
                    format_code_cell(&order_no),
                    format_code_cell(&sn),
                    format_yuan(amt),
                    time_part
                ));
            }
        }
        Ok(s)
    };

    let app_sec = format_refund_section(
        "5.1 家电电脑异常回款明细清单",
        app_refund_rows,
        &records.app_refund,
        headers[3],
    )?;
    out.push_str(&app_sec);

    let dig_sec = format_refund_section(
        "5.2 数码异常回款明细清单",
        dig_refund_rows,
        &records.dig_refund,
        headers[4],
    )?;
    out.push_str(&dig_sec);

    out.push_str("---\n\n");

    // ==========================================
    // 6. Abnormal Invoices
    // ==========================================
    out.push_str("## 六、 异常发票明细清单（黄色填充标识）\n\n");
    out.push_str("> **提取规则**：精准提取原始发票明细中单元格为黄色填充（#FFEB9C）且排除粉色填充的数据行。\n");
    out.push_str("> **开票属性说明**：清单内所有异常发票的【开票类型】全为【蓝票】，【开票状态】全为【开票完成】。\n");
    out.push_str("> **对账规则**：按 `匹配单据号` 归并档案，成对呈现同一单据号下开具的全部数电发票；组内按开票时间升序排列。\n\n");

    let inv_h = headers[2];
    let inv_no_idx = inv_h
        .find(&["数电发票号码"])
        .ok_or("发票明细缺少数电发票号码")?;
    let inv_name_idx = inv_h
        .find(&["主要商品名称"])
        .ok_or("发票明细缺少主要商品名称")?;
    let doc_no_idx = inv_h
        .find(&["匹配单据号"])
        .ok_or("发票明细缺少匹配单据号")?;
    let time_idx = inv_h.find(&["开票时间"]).unwrap_or(0);
    let type_idx = inv_h.find(&["开票类型"]).unwrap_or(1);
    let buyer_idx = inv_h.find(&["购方名称"]).unwrap_or(3);
    let desc_idx = inv_h.find(&["备注信息"]).unwrap_or(5);
    let status_idx = inv_h.find(&["开票状态"]).unwrap_or(6);

    let yellow_collected: Vec<_> = records
        .invoices
        .iter()
        .map(|&index| &invoices[index])
        .collect();

    let mut groups_map: HashMap<String, Vec<&Vec<Data>>> = HashMap::new();
    for row in &yellow_collected {
        let doc_no = cell_to_string(&row[doc_no_idx]);
        groups_map.entry(doc_no).or_default().push(row);
    }

    let mut groups: Vec<(String, Vec<&Vec<Data>>)> = groups_map.into_iter().collect();
    groups.sort_by(|(doc_a, _), (doc_b, _)| {
        doc_a
            .is_empty()
            .cmp(&doc_b.is_empty())
            .then_with(|| doc_a.cmp(doc_b))
    });

    out.push_str(&format!(
        "### 6.1 黄色异常发票明细清单（共 {} 组档案 · {} 笔发票）\n\n",
        groups.len(),
        yellow_collected.len()
    ));

    for (i, (doc_no, mut group_rows)) in groups.into_iter().enumerate() {
        group_rows.sort_by_key(|r| cell_to_string(&r[time_idx]));

        let prod_name = group_rows
            .iter()
            .map(|r| cell_to_string(&r[inv_name_idx]))
            .max_by_key(|n| n.len())
            .unwrap_or_default();
        let title_prod = if prod_name.is_empty() {
            String::new()
        } else {
            format!("{} · ", escape_md_cell(&prod_name))
        };

        out.push_str(&format!(
            "- **{}. 匹配单据号 {}**（{}共 {} 笔发票）\n```text\n",
            i + 1,
            format_code_cell(&doc_no),
            title_prod,
            group_rows.len()
        ));

        for (k, r) in group_rows.iter().enumerate() {
            let is_last_inv = k + 1 == group_rows.len();
            let inv_prefix = if is_last_inv {
                "└── "
            } else {
                "├── "
            };
            let child_prefix = if is_last_inv { "    " } else { "│   " };

            let inv_no = cell_to_string(&r[inv_no_idx]);
            let time = cell_to_string(&r[time_idx]);
            let buyer = cell_to_string(&r[buyer_idx]);
            let inv_type = cell_to_string(&r[type_idx]);
            let status = cell_to_string(&r[status_idx]);
            let desc = cell_to_string(&r[desc_idx]);

            out.push_str(&format!(
                "{}发票 {}：{}\n{}├── 开票档案：{} · 购方：{} · {} · {}\n{}└── 备注信息：{}\n",
                inv_prefix,
                k + 1,
                format_code_cell(&inv_no),
                child_prefix,
                escape_md_cell(&time),
                escape_md_cell(&buyer),
                escape_md_cell(&inv_type),
                escape_md_cell(&status),
                child_prefix,
                escape_md_cell(&desc)
            ));
        }

        out.push_str("```\n\n");
    }

    out.push_str("---\n\n");

    // ==========================================
    // 7. Methodology Notes
    // ==========================================
    out.push_str("## 七、 口径说明\n\n");
    out.push_str("1. **发生额与发生数量**：取自《销售用券情况统计.xlsx》；数量按“数量”字段净额统计，包含退货负数冲减。\n");
    out.push_str(
        "2. **品类归属口径**：“财务大类=数码”计入数码，其余所有财务大类统一计入家电电脑。\n",
    );
    out.push_str("3. **回款及上传状态**：已回款、审核通过未回款、待审核、审核失败取自两份“已上传”明细；金额汇总“补贴金额”，数量按明细记录数；审核失败包含审核终止。\n");
    out.push_str("4. **差额指标定义**：未回款 = 国补发生额 - 国补回款额；未上传 = 未回款 - 审核通过未回款 - 待审核 - 审核失败。\n");
    out.push_str("5. **比率分析算法**：回款率 = 国补回款额 / 国补发生额；综合回款率 = (国补回款额 + 审核通过未回款额) / 国补发生额。\n");
    out.push_str("6. **异常回款明细口径**：提取属于本店核销商编且在原始回款明细中单元格背景为粉色（#FFC7CE）的数据行；具有相同交易参考号的流水归并为冲销对档案，单独记录为独立扣减档案。\n");
    out.push_str("7. **异常发票明细口径**：提取原始发票明细中单元格为黄色填充（#FFEB9C）且排除粉色填充的数据行；具有相同匹配单据号的发票成对呈现，100% 完整保留原始开票属性与备注信息。\n");

    // ==========================================
    // Write out to file
    // ==========================================
    std::fs::write(output_path, out)
        .map_err(|e| format!("写入 Markdown 报告文件 {:?} 失败: {}", output_path, e))?;

    Ok(())
}

fn format_amount_count(amount: Decimal, count: i64, amount_unit: &str, count_unit: &str) -> String {
    let mut parts = Vec::new();
    if !amount.is_zero() {
        let formatted = if amount_unit == "万元" {
            format_wan(amount)
        } else {
            format_yuan(amount)
        };
        parts.push(format!("{}{}", formatted, amount_unit));
    }
    if count != 0 {
        parts.push(format!("{}{}", format_count(count), count_unit));
    }
    parts.join(" / ")
}

fn format_amount_ratio(amount: Decimal, pct: f64, rate_name: &str) -> String {
    let mut parts = vec![format!("{}万元", format_wan(amount))];
    if pct != 0.0 {
        parts.push(format!("{}：{}", rate_name, format_percent(pct)));
    }
    parts.join("，")
}

fn row_has_value(row: &MetricRow) -> bool {
    !row.app_amt.is_zero() || row.app_cnt != 0 || !row.dig_amt.is_zero() || row.dig_cnt != 0
}

fn push_metric_node(out: &mut String, prefix: &str, name: &str, row: &MetricRow) {
    let total = format_amount_count(row.tot_amt(), row.tot_cnt(), "万元", "笔");
    if total.is_empty() {
        out.push_str(&format!("{}{}\n", prefix, name));
    } else {
        out.push_str(&format!("{}{}（{}）\n", prefix, name, total));
    }
}

fn push_metric_details(out: &mut String, prefix: &str, row: &MetricRow) {
    for (kind, amount, count) in [
        ("家电", row.app_amt, row.app_cnt),
        ("数码", row.dig_amt, row.dig_cnt),
    ] {
        if !amount.is_zero() || count != 0 {
            out.push_str(&format!(
                "{}{}：{}\n",
                prefix,
                kind,
                format_amount_count(amount, count, "万元", "笔")
            ));
        }
    }
}

fn push_ratio_node(
    out: &mut String,
    prefix: &str,
    row: &(&str, Decimal, f64, Decimal, f64, Decimal, f64),
    rate_name: &str,
) {
    if row.5.is_zero() {
        out.push_str(&format!("{}{}\n", prefix, row.0));
    } else {
        out.push_str(&format!(
            "{}{}（{}）\n",
            prefix,
            row.0,
            format_amount_ratio(row.5, row.6, rate_name)
        ));
    }
}

fn push_ratio_details(
    out: &mut String,
    prefix: &str,
    row: &(&str, Decimal, f64, Decimal, f64, Decimal, f64),
    rate_name: &str,
) {
    for (kind, amount, pct) in [("家电", row.1, row.2), ("数码", row.3, row.4)] {
        if !amount.is_zero() {
            out.push_str(&format!(
                "{}{}：{}\n",
                prefix,
                kind,
                format_amount_ratio(amount, pct, rate_name)
            ));
        }
    }
}

struct CategoryOverview<'a> {
    name: &'a str,
    occur: Decimal,
    share: f64,
    paid: Decimal,
    pass: Decimal,
    wait: Decimal,
    fail: Decimal,
    unup: Decimal,
}

fn push_category_overview(out: &mut String, cat: &CategoryOverview) {
    if cat.occur.is_zero()
        && cat.paid.is_zero()
        && cat.pass.is_zero()
        && cat.wait.is_zero()
        && cat.fail.is_zero()
        && cat.unup.is_zero()
    {
        return;
    }

    let share_str = if cat.share != 0.0 {
        format!(" · 占比 {}", format_percent(cat.share))
    } else {
        String::new()
    };
    out.push_str(&format!(
        "- **{}**（发生 {} 万元{}）\n",
        cat.name,
        format_wan(cat.occur),
        share_str
    ));

    out.push_str("```text\n");

    let tot_unpaid = cat.pass + cat.wait + cat.fail + cat.unup;
    let paid_rate = div_pct(cat.paid, cat.occur);
    let unpaid_rate = div_pct(tot_unpaid, cat.occur);
    let comp_rate = div_pct(cat.paid + cat.pass, cat.occur);

    let has_paid = !cat.paid.is_zero();
    let has_unpaid = !tot_unpaid.is_zero();

    if has_paid {
        let prefix = if has_unpaid {
            "├── "
        } else {
            "└── "
        };
        out.push_str(&format!(
            "{}已回款：{} 万元（回款率 {}）\n",
            prefix,
            format_wan(cat.paid),
            format_percent(paid_rate)
        ));
    }

    if has_unpaid {
        let mut unpaid_details = Vec::new();
        if unpaid_rate != 0.0 {
            unpaid_details.push(format!("未回款率 {}", format_percent(unpaid_rate)));
        }
        if comp_rate != 0.0 {
            unpaid_details.push(format!("综合回款率 {}", format_percent(comp_rate)));
        }
        let rate_suffix = if unpaid_details.is_empty() {
            String::new()
        } else {
            format!("（{}）", unpaid_details.join(" · "))
        };
        out.push_str(&format!(
            "└── 未回款：{} 万元{}\n",
            format_wan(tot_unpaid),
            rate_suffix
        ));

        let mut children: Vec<(&str, Decimal)> = Vec::new();
        if !cat.pass.is_zero() {
            children.push(("审核通过未回款", cat.pass));
        }
        if !cat.wait.is_zero() {
            children.push(("待审核", cat.wait));
        }
        if !cat.fail.is_zero() {
            children.push(("审核失败", cat.fail));
        }
        if !cat.unup.is_zero() {
            children.push(("未上传", cat.unup));
        }

        for (i, (c_name, amt)) in children.iter().enumerate() {
            let c_prefix = if i + 1 == children.len() {
                "    └── "
            } else {
                "    ├── "
            };
            out.push_str(&format!(
                "{}{}：{} 万元\n",
                c_prefix,
                c_name,
                format_wan(*amt)
            ));
        }
    }

    out.push_str("```\n\n");
}

fn push_counter_entry(
    out: &mut String,
    name: &str,
    amount: Decimal,
    count: i64,
    statuses: &[&str; 5],
    values: &[(Decimal, i64); 5],
) {
    if amount.is_zero() && count == 0 && values.iter().all(|(amt, cnt)| amt.is_zero() && *cnt == 0)
    {
        return;
    }

    let amt_cnt_str = format_amount_count(amount, count, " 元", " 件");
    out.push_str(&format!(
        "- **{}**（发生合计 {}）\n",
        escape_md_cell(name),
        amt_cnt_str
    ));

    out.push_str("```text\n");

    let paid = values[0];
    let pass = values[1];
    let wait = values[2];
    let fail = values[3];
    let unup = values[4];

    let tot_unpaid_amt = pass.0 + wait.0 + fail.0 + unup.0;
    let tot_unpaid_cnt = pass.1 + wait.1 + fail.1 + unup.1;

    let has_paid = !paid.0.is_zero() || paid.1 != 0;
    let has_unpaid = !tot_unpaid_amt.is_zero() || tot_unpaid_cnt != 0;

    if has_paid {
        let prefix = if has_unpaid {
            "├── "
        } else {
            "└── "
        };
        out.push_str(&format!(
            "{}已回款：{}\n",
            prefix,
            format_amount_count(paid.0, paid.1, " 元", " 件")
        ));
    }

    if has_unpaid {
        out.push_str(&format!(
            "└── 未回款：{}\n",
            format_amount_count(tot_unpaid_amt, tot_unpaid_cnt, " 元", " 件")
        ));

        let unpaid_statuses = [
            (statuses[1], pass),
            (statuses[2], wait),
            (statuses[3], fail),
            (statuses[4], unup),
        ];

        let visible_unpaid: Vec<_> = unpaid_statuses
            .iter()
            .filter(|(_, (amt, cnt))| !amt.is_zero() || *cnt != 0)
            .collect();

        for (i, (s_name, (amt, cnt))) in visible_unpaid.iter().enumerate() {
            let c_prefix = if i + 1 == visible_unpaid.len() {
                "    └── "
            } else {
                "    ├── "
            };
            out.push_str(&format!(
                "{}{}：{}\n",
                c_prefix,
                s_name,
                format_amount_count(*amt, *cnt, " 元", " 件")
            ));
        }
    }

    out.push_str("```\n\n");
}

fn format_wan(d: Decimal) -> String {
    let wan = d / Decimal::from(10000);
    format_thousands_decimal(wan, 2)
}

fn format_yuan(d: Decimal) -> String {
    format_thousands_decimal(d, 2)
}

fn format_count(cnt: i64) -> String {
    format_thousands_decimal(Decimal::from(cnt), 0)
}

fn format_percent(pct: f64) -> String {
    format!("{:.2}%", pct * 100.0)
}

fn format_thousands_decimal(d: Decimal, scale: u32) -> String {
    let rounded = d.round_dp(scale);
    let is_negative = rounded < Decimal::ZERO;
    let abs_d = rounded.abs();
    let s = format!("{:.scale$}", abs_d, scale = scale as usize);
    let (int_part, frac_part) = s.split_once('.').unwrap_or((&s, ""));

    let bytes = int_part.as_bytes();
    let mut formatted = String::with_capacity(s.len() + bytes.len() / 3 + 1);
    if is_negative {
        formatted.push('-');
    }
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(b as char);
    }
    if !frac_part.is_empty() {
        formatted.push('.');
        formatted.push_str(frac_part);
    }
    formatted
}

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

fn format_date_str(cell: &Data) -> String {
    let s = cell_to_string(cell);
    if let Some((d, _)) = s.split_once(' ') {
        d.to_string()
    } else {
        s
    }
}

fn format_code_cell(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "-" {
        "-".to_string()
    } else {
        format!("`{}`", escape_md_cell(trimmed).replace('`', ""))
    }
}

fn map_counter_name(cat: &str, brand: &str) -> String {
    let clean_cat = cat.trim();
    let clean_brand = brand.trim();
    if clean_cat == "数码" {
        return "数码".to_string();
    }
    const BRAND_ONLY: [&str; 10] = [
        "AO史密斯",
        "万家乐",
        "方太",
        "欧意",
        "老板",
        "小鸭",
        "奥克斯",
        "格力",
        "科龙",
        "沁园",
    ];
    if BRAND_ONLY.contains(&clean_brand) {
        clean_brand.to_string()
    } else {
        format!("{}{}", clean_brand, clean_cat)
    }
}
