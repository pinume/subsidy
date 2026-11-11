use std::process::Command;

#[test]
fn comparison_detects_missing_extra_and_changed_files() {
    let root = std::env::temp_dir().join(format!("comparison-{}", std::process::id()));
    for side in ["old", "new"] {
        for folder in ["国补明细", "国补报表"] {
            std::fs::create_dir_all(root.join(side).join(folder)).unwrap();
        }
    }
    let compare = || {
        Command::new("bash")
            .arg(format!(
                "{}/scripts/compare-outputs.sh",
                env!("CARGO_MANIFEST_DIR")
            ))
            .arg(root.join("old"))
            .arg(root.join("new"))
            .output()
            .unwrap()
    };
    assert!(compare().status.success());
    let old = root.join("old/国补报表/test.xlsx");
    let new = root.join("new/国补报表/test.xlsx");
    let write = |path: &std::path::Path, value: &str| {
        let mut book = rust_xlsxwriter::Workbook::new();
        book.add_worksheet().write_string(0, 0, value).unwrap();
        book.save(path).unwrap();
    };
    write(&old, "same");
    let result = compare();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stdout).contains("仅旧运行存在"));
    write(&new, "same");
    assert!(compare().status.success());
    write(&new, "changed");
    assert_eq!(compare().status.code(), Some(1));
    std::fs::remove_file(old).unwrap();
    let result = compare();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stdout).contains("仅新运行存在"));
    std::fs::remove_dir_all(root).unwrap();
}
