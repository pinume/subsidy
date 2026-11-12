# subsidy 项目架构

业务口径分别以[清洗规则](cleaning-rules.md)和[报表规则](reporting-rules.md)为准。

## 模块边界

| 模块 | 职责与边界 |
|---|---|
| `app/cli.rs` | 读取、校验源路径；交互使用标准库 |
| `app/runner.rs` | 运行锁、8类清洗调度、逐类发布与失败隔离 |
| `app/reports.rs` | 按本次发布状态检查两份工作簿的依赖；隔离4份报表的生成与发布 |
| `jobs/` | 文件识别、结构校验、字段转换、匹配和填色；返回 `Table`，由调度器发布 |
| `model/` | 表、行、值、列类型、填色与错误；不依赖任务和文件 I/O |
| `utils/` | 日期、十进制金额、文本、单据号等共用规则；不访问文件系统 |
| `io/paths.rs` | 文件识别、输出目录和真实路径保护 |
| `io/xlsx_reader.rs` | 保留源单元格类型，以 `Range::start()` 换算 Excel 绝对坐标 |
| `io/xlsx_writer.rs` | 校验 `Table` 列数，按列类型写单表 XLSX；业务填色优先于斑马纹 |
| `io/publisher.rs` | 同目录临时文件、替换与清理；失败保留旧结果 |
| `reporting/data.rs`、`reader.rs` | 标准明细读取、表头与类型映射、商户号及异常底色解析 |
| `reporting/summary_data.rs` | 汇总指标、品类品牌聚合与排序、异常行索引 |
| `reporting/summary_wb.rs`、`finance_wb.rs` | 分别生成汇总和财务统筹工作簿 |
| `reporting/ledger.rs` | 家电、数码台账：读取原始补贴明细，按原始发票填写红冲状态，家电另按发票补空 |
| `reporting/excel.rs`、`styles.rs` | 统一排版、类型写入和 `ReportError`；公开报表接口返回 `String` 错误 |

## 规则所有者

- 单据号提取、补零、歧义 → `MatchDocNo` (`src/utils/doc_no.rs`)
- 品牌、财务大类归并 → `ProductIdentity` (`src/utils/product.rs`)
- 金额舍入（逢五进一）→ `round_cents` (`src/utils/numbers.rs`)
- 重复导出取 `更新时间` 最新 → `latest_by_key` (`src/utils/latest.rs`)，清洗与报表共用
- 多候选与歧义判定 → `MultiValueIndex` / `PriorityOutcome` (`src/jobs/multi_value_index.rs`)
- 蓝票取号 (清洗 §10.10 及财务兜底) → `BlueInvoiceCandidates` (`src/jobs/invoice.rs`)
- 上传状态匹配 (§10.12.2) → `UploadStatusIndex` (`src/jobs/uploaded.rs`)
- 回款补贴匹配 → `RefundSubsidyIndex` (`src/jobs/refund.rs`)
- 回款源表读取（文件选择、`汇总`排除、表头定位与去空白）→ `RefundJob::batch_sheets` / `BatchSheet` (`src/jobs/refund.rs`)，回款清洗与台账共用
- 销售用券备注级联、匹配统计 → `SalesCouponReconciler` (`src/jobs/coupons.rs`)
- 清洗中间表与索引缓存 → `PipelineContext` (`src/jobs/context.rs`)；整表共享使用 `Job::run_shared`
- 财务统筹最终匹配 → `TransactionReconciler` (`src/reporting/reconciler.rs`)
- 报表业务键冲突与告警 → `UniqueIndex` (`src/reporting/unique_index.rs`)
- 汇总指标、上传去重、异常明细 → `src/reporting/summary_data.rs`
- 工作表排版 → `ReportSheetBuilder`; 写出错误 → `ReportError` (`src/reporting/excel.rs`)

## 执行与共享

```text
源目录校验 → 国补明细/.combine.lock 文件锁
  → 按 registry 顺序清洗、校验、逐文件发布8类明细
  → 按本次发布状态检查报表依赖
  → 加载6份共同明细 → 汇总工作簿
  → 复用共同明细，额外读取银联明细 → 财务统筹工作簿
  → 读取原始补贴明细及最新原始发票 → 家电台账、数码台账
  → 逐文件发布、汇总退出状态、释放锁
```

清洗通过 `PipelineContext` 显式传递索引和中间表。已上传依赖对应回款数据，销售用券依赖银联、发票、收款单及两份已上传数据。已上传和回款的 `Job::run_shared()` 通过 `cached_table()` 复用 `Rc<Table>`；索引按需构建，只缓存成功结果，运行结束释放。被引用任务只提供数据，发布仍由调度器负责。

CLI 报表阶段使用局部 `OnceCell<Result<CommonInputs, String>>` 加载6份共同明细及商户号。财务统筹额外读取银联，完整成功流程共解析7份输入。异常行保存为源行索引，渲染借用共同数据。共同输入加载失败影响两份报表，单份报表的底色读取、计算、写出或发布失败只影响自身。台账不检查清洗依赖，直接读取原始数据：回款源表的文件选择、`汇总`排除与表头定位复用 `RefundJob::batch_sheets`，发票复用 `invoice::load_records`（`合计含税金额` 仅供台账，不进入清洗输出）；不缓存到 `PipelineContext`。

库导出 `generate_summary_workbook` 和 `generate_store_finance_workbook`；独立调用自行加载共同输入，CLI 使用内部共享入口。

## Excel 边界

清洗后的 XLSX 是清洗与报表的边界，日期类型和异常颜色从文件读取。原始表与标准明细使用不同读取器，分别维护各自的数据契约。

金额清洗与计算使用 `Decimal`；写入 XLSX 数值单元格时转为 `f64`，报表读取时用 `Decimal::from_f64` 转回。写出不因 `0.00` 显示格式而舍入。`from_f64` 约保留15位有效数字；`from_f64_retain` 会保留二进制尾差，影响零值判断、零分母比率和短横线显示，因此由 `clippy.toml` 禁用。真实 XLSX 往返契约由 `src/reporting/float_roundtrip_tests.rs` 验证。

## 运行与验证

运行入口与构建命令见 [README](../README.md)。有效路径去除首尾空白及成对引号，相对路径基于启动目录；无效输入重新提示，EOF 正常退出。全部成功（含已发布但有冲突告警）或 EOF 退出码为 `0`；清洗失败、报表失败、依赖缺失跳过报表或无法开始时为 `1`。

源文件只读。两处输出目录由源目录的父目录推导；真实路径与源目录或彼此重叠时拒绝处理。全流程持有标准库文件锁，进程退出由操作系统释放，锁文件保留。

每个结果先写同目录唯一临时文件，再用 rename 替换正式文件。失败保留该旧文件，清理守卫覆盖提前返回及 panic；清理失败报告原始错误和残留路径。本次未成功发布的类别不能以旧文件满足报表依赖。4份报表独立发布，无跨文件事务。

普通测试覆盖业务规则、路径与锁、发布失败、共享输入单次读取及 CLI；`tests/full_pipeline.rs` 用小型 Excel 夹具验证8类清洗与4份报表。真实批次的对比与验收见[真实数据核对](agents/real-data.md)。
