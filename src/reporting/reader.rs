use calamine::{Data, DataType, Reader, open_workbook_auto};
use quick_xml::Reader as XmlReader;
use quick_xml::events::Event;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use zip::ZipArchive;

/// Read all rows from the first sheet of an Excel file.
pub fn read_sheet_rows(path: &Path) -> Result<Vec<Vec<Data>>, String> {
    let mut workbook =
        open_workbook_auto(path).map_err(|e| format!("打开文件 {:?} 失败: {}", path, e))?;
    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err(format!("文件 {:?} 中未找到任何工作表", path));
    }
    let range = workbook
        .worksheet_range(&sheet_names[0])
        .map_err(|e| format!("读取工作表 {} 失败: {}", sheet_names[0], e))?;

    let rows: Vec<Vec<Data>> = range.rows().map(|r| r.to_vec()).collect();
    if rows.first().is_none_or(Vec::is_empty) {
        return Err(format!("文件 {:?} 的首个工作表为空", path));
    }
    #[cfg(test)]
    READ_PATHS.with(|paths| paths.borrow_mut().push(path.to_path_buf()));
    Ok(rows)
}

#[cfg(test)]
thread_local! {
    static READ_PATHS: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn take_read_paths() -> Vec<std::path::PathBuf> {
    READ_PATHS.with(|paths| std::mem::take(&mut *paths.borrow_mut()))
}

#[cfg(test)]
pub fn read_upload_rows(path: &Path) -> Result<Vec<Vec<Data>>, String> {
    super::data::SheetData::load(path, true).map(|sheet| sheet.to_vec())
}

/// Dynamic header mapper for finding columns by name or alias.
#[derive(Debug, Clone)]
pub struct HeaderMap {
    // column name (lowercase) -> list of 0-based column indices
    map: HashMap<String, Vec<usize>>,
}

impl HeaderMap {
    pub fn from_header_row(row: &[Data]) -> Self {
        let mut map: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, cell) in row.iter().enumerate() {
            let name = cell.to_string().trim().to_string();
            if !name.is_empty() {
                map.entry(name.to_lowercase()).or_default().push(idx);
            }
        }
        Self { map }
    }

    /// Find single column index by exact name or aliases
    pub fn find(&self, names: &[&str]) -> Option<usize> {
        for name in names {
            let key = name.trim().to_lowercase();
            if let Some(first) = self.map.get(&key).and_then(|ids| ids.first().copied()) {
                return Some(first);
            }
        }
        None
    }

    /// Find N-th occurrence of a column name (1-based occurrence)
    pub fn find_occurrence(&self, name: &str, occurrence: usize) -> Option<usize> {
        let key = name.trim().to_lowercase();
        self.map
            .get(&key)
            .and_then(|ids| ids.get(occurrence.checked_sub(1)?).copied())
    }

    /// Require a column name, returning error if not found
    pub fn require(&self, name: &str, file_label: &str) -> Result<usize, String> {
        self.find(&[name])
            .ok_or_else(|| format!("{}: 缺少必要列 [{}]", file_label, name))
    }
}

/// Frequently accessed column indices in uploaded spreadsheets
#[derive(Debug, Clone, Copy)]
pub struct UploadColumns {
    pub status: usize,
    pub subsidy: usize,
    pub reference: usize,
    pub invoice: usize,
}

impl UploadColumns {
    pub fn from_header(h: &HeaderMap, file_label: &str) -> Result<Self, String> {
        Ok(Self {
            status: h.require("状态", file_label)?,
            subsidy: h.require("补贴金额", file_label)?,
            reference: h.require("检索参考号", file_label)?,
            invoice: h.require("发票号码", file_label)?,
        })
    }
}

/// Extract unique non-empty merchant code from rows, ensuring consistency across data rows
pub fn extract_unique_merchant_code(
    rows: &[Vec<Data>],
    header: &HeaderMap,
    file_label: &str,
) -> Result<String, String> {
    if rows.len() < 2 {
        return Err(format!("{}: 数据为空", file_label));
    }
    let mch_idx = header.require("商户号", file_label)?;

    let mut found_code = None;
    for (r_idx, row) in rows.iter().enumerate().skip(1) {
        if mch_idx < row.len() {
            let code = cell_to_string(&row[mch_idx]);
            if !code.is_empty() {
                match &found_code {
                    None => found_code = Some(code),
                    Some(first) if first != &code => {
                        return Err(format!(
                            "{}: 第 {} 行商户号 '{}' 与首个商户号 '{}' 不一致，存在歧义",
                            file_label,
                            r_idx + 1,
                            code,
                            first
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    found_code.ok_or_else(|| format!("{}: 未找到任何有效商户号", file_label))
}

/// Convert calamine cell data to clean String
pub fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Data::Int(i) => format!("{}", i),
        Data::Bool(b) => format!("{}", b),
        Data::DateTime(dt) => {
            if let Some(chrono_dt) = cell.as_datetime() {
                chrono_dt.format("%Y-%m-%d %H:%M:%S").to_string()
            } else {
                format!("{}", dt)
            }
        }
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR:{:?}", e),
    }
}

/// Convert calamine cell data to Decimal for high precision money calculation
pub fn cell_to_decimal(cell: &Data) -> Option<Decimal> {
    match cell {
        Data::Empty => None,
        Data::Int(i) => Some(Decimal::from(*i)),
        Data::Float(f) => Decimal::from_f64_retain(*f),
        Data::String(s) => {
            let s_clean = s.trim().replace(',', "");
            Decimal::from_str_exact(&s_clean).ok()
        }
        _ => None,
    }
}

/// Extract 1-indexed row numbers that have cell fill color matching any of `target_hexes` in a single pass.
/// Returns a map of cleaned uppercase hex string -> Set of row numbers.
pub fn get_colored_row_indices_multi(
    path: &Path,
    target_hexes: &[&str],
) -> Result<HashMap<String, HashSet<u32>>, String> {
    let file = File::open(path).map_err(|e| format!("打开 {:?} 失败: {}", path, e))?;
    let mut zip = ZipArchive::new(BufReader::new(file))
        .map_err(|e| format!("解压 {:?} 失败: {}", path, e))?;

    let clean_targets: Vec<String> = target_hexes
        .iter()
        .map(|h| h.trim_start_matches('#').to_uppercase())
        .collect();

    // 1. Parse xl/styles.xml in separate scope so zip can be reused
    let xf_to_targets = {
        let mut styles_file = zip
            .by_name("xl/styles.xml")
            .map_err(|e| format!("读取 xl/styles.xml 失败: {}", e))?;

        let mut xml_buf = Vec::new();
        let mut reader = XmlReader::from_reader(BufReader::new(&mut styles_file));
        reader.config_mut().trim_text(true);

        let mut in_fills = false;
        let mut fills: Vec<Option<String>> = Vec::new();
        let mut in_cell_xfs = false;
        let mut xf_fill_ids: Vec<usize> = Vec::new();

        let mut current_fill_color: Option<String> = None;

        loop {
            match reader.read_event_into(&mut xml_buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"fills" => in_fills = true,
                    b"fill" if in_fills => {
                        current_fill_color = None;
                    }
                    b"fgColor" if in_fills => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"rgb" {
                                let val = String::from_utf8_lossy(&attr.value).to_uppercase();
                                current_fill_color = Some(val);
                            }
                        }
                    }
                    b"cellXfs" => in_cell_xfs = true,
                    b"xf" if in_cell_xfs => {
                        let mut fill_id = 0;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"fillId"
                                && let Ok(id) = std::str::from_utf8(&attr.value)
                                    .unwrap_or("0")
                                    .parse::<usize>()
                            {
                                fill_id = id;
                            }
                        }
                        xf_fill_ids.push(fill_id);
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => match e.name().as_ref() {
                    b"fills" => in_fills = false,
                    b"fill" if in_fills => {
                        fills.push(current_fill_color.take());
                    }
                    b"cellXfs" => in_cell_xfs = false,
                    _ => {}
                },
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("解析 styles.xml 失败: {}", e)),
                _ => {}
            }
            xml_buf.clear();
        }

        // Map XF index to matched target hex strings
        let mut xf_map: HashMap<usize, Vec<String>> = HashMap::new();
        for (xf_idx, &fill_id) in xf_fill_ids.iter().enumerate() {
            if let Some(Some(color)) = fills.get(fill_id) {
                for target in &clean_targets {
                    if color == target || color.ends_with(target) || target.ends_with(color) {
                        xf_map.entry(xf_idx).or_default().push(target.clone());
                    }
                }
            }
        }
        xf_map
    };

    let mut result_map: HashMap<String, HashSet<u32>> = clean_targets
        .iter()
        .map(|t| (t.clone(), HashSet::new()))
        .collect();

    if xf_to_targets.is_empty() {
        return Ok(result_map);
    }

    // 2. Parse xl/worksheets/sheet1.xml in a single pass
    let mut sheet_file = zip
        .by_name("xl/worksheets/sheet1.xml")
        .map_err(|e| format!("读取 xl/worksheets/sheet1.xml 失败: {}", e))?;

    let mut xml_buf = Vec::new();
    let mut sheet_reader = XmlReader::from_reader(BufReader::new(&mut sheet_file));
    sheet_reader.config_mut().trim_text(true);

    let mut current_row: u32 = 0;

    loop {
        match sheet_reader.read_event_into(&mut xml_buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                b"row" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"r"
                            && let Ok(r) = std::str::from_utf8(&attr.value)
                                .unwrap_or("0")
                                .parse::<u32>()
                        {
                            current_row = r;
                        }
                    }
                }
                b"c" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"s"
                            && let Ok(s) = std::str::from_utf8(&attr.value)
                                .unwrap_or("0")
                                .parse::<usize>()
                            && let Some(targets) = xf_to_targets.get(&s)
                            && current_row > 0
                        {
                            for target in targets {
                                if let Some(set) = result_map.get_mut(target) {
                                    set.insert(current_row);
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("解析 sheet1.xml 失败: {}", e)),
            _ => {}
        }
        xml_buf.clear();
    }

    Ok(result_map)
}

/// Extract 1-indexed row numbers that have cell fill color matching `target_hex`
pub fn get_colored_row_indices(path: &Path, target_hex: &str) -> Result<HashSet<u32>, String> {
    let clean = target_hex.trim_start_matches('#').to_uppercase();
    let mut map = get_colored_row_indices_multi(path, &[target_hex])?;
    Ok(map.remove(&clean).unwrap_or_default())
}
