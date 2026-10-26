//! Incremental recalculation through defined names and structured references gives the cell
//! values a full recalculation gives, whatever is edited: cells a name or table covers, cells
//! beside them, names (edited, renamed, deleted) and tables (resized, grown). Names and tables
//! are changed both the way the engine does (followed by `recalc_all`) and without any rebuild
//! (followed only by `cells_changed` of an unrelated cell), which `Calc` must notice itself.
//!
//! `GRIDCRAFT_ORACLE_DUMP=<file>` also writes one hash per step, to compare two builds.

use std::fmt::Write as _;
use std::sync::Arc;

use gridcraft_calc::Calc;
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{Cell, DefinedName, Formula, Sheet, Table, TableColumn, Workbook};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

/// (name, scope, body). Bodies cover constants, cells, ranges, nesting, relative references,
/// sheet-scoped names, unqualified references (they follow the formula's sheet), table names, and
/// the ones that must stay dynamic or error: OFFSET, a cycle, text that doesn't parse.
const NAMES: &[(&str, Option<usize>, &str)] = &[
    ("Rate", None, "Sheet1!$H$1"),
    ("Rate", None, "Sheet1!$H$2"),
    ("Rate", None, "0.5"),
    ("Rate", Some(0), "Sheet1!$H$3"),
    ("Rng", None, "Sheet1!$A$1:$A$8"),
    ("Rng", None, "Sheet1!$A$1:$A$4"),
    ("Cst", None, "0.25"),
    ("Cst", None, "Sheet1!$H$2*2"),
    ("Nest", None, "Rate*2+Cst"),
    ("Cond", None, "IF(Sheet1!$H$1>3,Sheet1!$H$1*2+Sheet1!$H$2,ROUND(Sheet1!$H$3,2))"),
    ("Tab", None, "T1"),
    ("Tab", None, "T2"),
    ("T3", None, "T1"),
    ("Rel", None, "Sheet1!B1"),
    ("Dyn", None, "OFFSET(Sheet1!$H$1,1,0)"),
    ("Loc", Some(1), "Sheet2!$A$1"),
    ("Loc", None, "Sheet2!$A$2"),
    ("Unq", None, "$A$1"),
    ("Same", None, "Same+1"),
    ("Bad", None, "(("),
    ("Chain", None, "Nest+Cond"),
    ("Spl", None, "Sheet1!$M$1#"),
    // Over the cells the formulas themselves are in: chains, and cycles through a name.
    ("Blk", None, "Sheet1!$B$1:$B$6"),
    ("Cc", None, "Sheet1!$C$3"),
    ("Dd", Some(1), "Sheet2!$B$2:$C$3"),
];

const FORMULAS: &[&str] = &[
    "=Rate*A{r}",
    "=SUM(Rng)+A{r}",
    "=Nest+A{r}",
    "=Cond",
    "=SUM(Tab)",
    "=SUM(T1[Price])+{r}",
    "=ROWS(T1)",
    "=SUM(T1)",
    "=COUNT(T1[[#Headers],[Qty]])",
    "=INDEX(Rng,2)",
    "=SUMIF(Rng,\">1\")",
    "=Rel",
    "=Dyn",
    "=Loc*2",
    "=Sheet2!Loc",
    "=Unq",
    "=Same",
    "=Bad",
    "=Missing+1",
    "=LET(Rate,5,Rate*A{r})",
    "=Chain",
    "=SUM(Spl)",
    "=ROWS(Spl)",
    "=SUM(T1[Qty])*Rate",
    "=A{r}:Rate",
    "=MAX(Tab,Rng)",
    "=T2[Z]+1",
    "=SUM(Blk)+1",
    "=Cc+A{r}",
    "=SUM(Dd)",
    "=Cc*Rate+SUM(Blk)",
];

const TABLE_FORMULAS: &[&str] = &["=[@Qty]*[@Price]", "=[@Qty]+Rate", "=SUM(T1[Qty])-[@Price]", "=[@Qty]*Cst"];

fn column(name: &str) -> TableColumn {
    TableColumn { name: name.into(), totals: Default::default(), totals_label: None, formula: None, unique_name: None, xml: None }
}

fn table(id: u32, name: &str, range: &str, cols: &[&str]) -> Table {
    Table {
        id,
        name: name.into(),
        range: RangeRef::parse(range).unwrap(),
        header_row: true,
        totals_row: false,
        columns: cols.iter().map(|c| column(c)).collect(),
        style: String::new(),
        banded_rows: false,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: false,
        filter: vec![],
        table_type: None,
        connection_id: None,
    }
}

fn c(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}

fn num(wb: &mut Workbook, sheet: usize, at: CellRef, v: f64) {
    wb.sheet_mut(sheet).unwrap().cells.set(at, Cell::value(Value::Number(v)));
}

fn formula(wb: &mut Workbook, sheet: usize, at: CellRef, text: &str) {
    wb.sheet_mut(sheet).unwrap().cells.set(at, Cell::formula(Formula::new(text)));
}

fn start() -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets.push(Arc::new(Sheet::new("Sheet2")));
    for r in 0..12 {
        num(&mut wb, 0, CellRef::new(r, 0), (r + 1) as f64);
    }
    for r in 0..3 {
        num(&mut wb, 0, CellRef::new(r, 7), 2.0 + r as f64);
    }
    for r in 0..5 {
        num(&mut wb, 1, CellRef::new(r, 0), 10.0 * (r + 1) as f64);
    }
    // T1 on Sheet1, J1:L8: Qty, Price and a calculated column. T2 on Sheet2, D1:E4.
    wb.sheet_mut(0).unwrap().tables.push(table(1, "T1", "J1:L8", &["Qty", "Price", "Calc"]));
    wb.sheet_mut(1).unwrap().tables.push(table(2, "T2", "D1:E4", &["Z", "Y"]));
    for (at, text) in [("J1", "Qty"), ("K1", "Price"), ("L1", "Calc")] {
        wb.sheet_mut(0).unwrap().cells.set(c(at), Cell::value(Value::text(text)));
    }
    for r in 1..8 {
        num(&mut wb, 0, CellRef::new(r, 9), r as f64);
        num(&mut wb, 0, CellRef::new(r, 10), 3.0 * r as f64);
        formula(&mut wb, 0, CellRef::new(r, 11), TABLE_FORMULAS[0]);
    }
    for (at, text) in [("D1", "Z"), ("E1", "Y")] {
        wb.sheet_mut(1).unwrap().cells.set(c(at), Cell::value(Value::text(text)));
    }
    for r in 1..4 {
        num(&mut wb, 1, CellRef::new(r, 3), r as f64);
        num(&mut wb, 1, CellRef::new(r, 4), 2.0 * r as f64);
    }
    formula(&mut wb, 0, c("M1"), "=SEQUENCE(MAX(1,Sheet1!$H$1))");
    wb
}

/// Every cell value (and spilled value) of the workbook, as text, sheet by sheet in cell order.
fn snapshot(wb: &Workbook) -> String {
    let mut out = String::new();
    for (si, sh) in wb.sheets.iter().enumerate() {
        let mut rows: Vec<(u32, u32, String)> = sh.cells.iter().map(|(at, cell)| (at.row, at.col, format!("{:?}", cell.value))).collect();
        rows.extend(sh.spill.iter().map(|(at, v)| (at.row, at.col, format!("spill {v:?}"))));
        rows.sort();
        for (r, col, v) in rows {
            let _ = writeln!(out, "{si}!{r},{col}={v}");
        }
    }
    out
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn random_formula(rng: &mut Rng) -> String {
    rng.pick(FORMULAS).replace("{r}", &(rng.below(10) + 1).to_string())
}

/// A formula and where it goes: beside the data, or inside T1 (calculated-column style).
fn random_placed(rng: &mut Rng) -> (usize, CellRef, String) {
    if rng.below(3) == 0 {
        let at = CellRef::new(1 + rng.below(8) as u32, 11);
        let text = if rng.below(2) == 0 { rng.pick(TABLE_FORMULAS).to_string() } else { random_formula(rng) };
        return (0, at, text);
    }
    let sheet = rng.below(2);
    (sheet, CellRef::new(rng.below(14) as u32, 1 + rng.below(3) as u32), random_formula(rng))
}

/// Some cell near what names and tables cover (and some far from it).
fn random_cell(rng: &mut Rng) -> (usize, CellRef) {
    match rng.below(8) {
        0 | 1 => (0, CellRef::new(rng.below(12) as u32, 0)),
        2 => (0, CellRef::new(rng.below(4) as u32, 7)),
        3 | 4 => (0, CellRef::new(rng.below(13) as u32, 9 + rng.below(2) as u32)),
        5 => (1, CellRef::new(rng.below(6) as u32, 0)),
        6 => (1, CellRef::new(rng.below(6) as u32, 3 + rng.below(2) as u32)),
        _ => (0, CellRef::new(30 + rng.below(5) as u32, 20)),
    }
}

fn run(seed: u64, steps: usize, dump: &mut String) {
    let mut rng = Rng(seed);
    let mut wb = start();
    let mut calc = Calc::new();
    calc.recalc_all(&mut wb);
    // Formulas to start with: spread over both sheets, beside and inside the table.
    for _ in 0..30 {
        let (sheet, at, text) = random_placed(&mut rng);
        formula(&mut wb, sheet, at, &text);
    }
    for (name, scope, body) in NAMES.iter().take(3) {
        wb.names.push(DefinedName { name: (*name).into(), scope: *scope, formula: (*body).into(), comment: String::new(), hidden: false });
    }
    calc.recalc_all(&mut wb);
    for step in 0..steps {
        let mut changed: Vec<(usize, CellRef)> = Vec::new();
        let what = rng.below(100);
        let mut what_text = String::new();
        if what < 8 {
            // Every input cell in turn, one edit each: a formula that misses a precedent keeps a stale value.
            let mut cells = vec![];
            for r in 1..13u32 {
                cells.extend([(0, CellRef::new(r, 0)), (0, CellRef::new(r, 9)), (0, CellRef::new(r, 10))]);
            }
            cells.extend((0..3u32).map(|r| (0, CellRef::new(r, 7))));
            cells.extend((0..5u32).map(|r| (1, CellRef::new(r, 0))));
            cells.extend((1..5u32).map(|r| (1, CellRef::new(r, 3))));
            for (s, at) in cells {
                num(&mut wb, s, at, rng.below(9) as f64 - 2.0);
                calc.cells_changed(&mut wb, &[(s, at)]);
                let mut fresh = wb.clone();
                Calc::new().recalc_all(&mut fresh);
                let (inc, full) = (snapshot(&wb), snapshot(&fresh));
                let _ = writeln!(dump, "{seed} {step} sweep {s}:{},{} {:016x}", at.row, at.col, hash(&inc));
                assert!(
                    inc == full,
                    "seed {seed} step {step} (sweep, edit of {s}!{},{}): incremental differs from full recalculation",
                    at.row,
                    at.col
                );
            }
            what_text.push_str("sweep");
        } else if what < 30 {
            let (s, at) = random_cell(&mut rng);
            num(&mut wb, s, at, rng.below(9) as f64 - 2.0);
            changed.push((s, at));
            what_text.push_str("number");
        } else if what < 45 {
            let (s, at, text) = random_placed(&mut rng);
            formula(&mut wb, s, at, &text);
            changed.push((s, at));
            what_text.push_str("formula");
        } else if what < 50 {
            let (s, at) = random_cell(&mut rng);
            wb.sheet_mut(s).unwrap().cells.remove(at);
            changed.push((s, at));
            what_text.push_str("clear");
        } else if what < 66 {
            // Define (or redefine) a name.
            let (name, scope, body) = *rng.pick(NAMES);
            wb.names.retain(|n| !(n.name.eq_ignore_ascii_case(name) && n.scope == scope));
            wb.names.push(DefinedName { name: name.into(), scope, formula: body.into(), comment: String::new(), hidden: false });
            what_text.push_str("define");
        } else if what < 74 {
            if !wb.names.is_empty() {
                let i = rng.below(wb.names.len());
                wb.names.remove(i);
            }
            what_text.push_str("delete");
        } else if what < 80 {
            // Rename: the formulas keep the old name (the engine rewrites them; here they go #NAME?).
            if !wb.names.is_empty() {
                let i = rng.below(wb.names.len());
                let other = rng.pick(NAMES).0;
                if let Some(n) = wb.names.get_mut(i) {
                    n.name = other.into();
                }
            }
            what_text.push_str("rename");
        } else if what < 92 {
            // Resize or grow T1 or T2 (the range, header or totals row, columns).
            let (s, ti) = (rng.below(2), 0);
            if let Some(t) = wb.sheet_mut(s).and_then(|sh| sh.tables.get_mut(ti)) {
                match rng.below(4) {
                    0 => t.range.end.row = (t.range.end.row + 1).min(14),
                    1 => t.range.end.row = t.range.end.row.saturating_sub(1).max(t.range.start.row + 1),
                    2 => t.totals_row = !t.totals_row,
                    _ => t.header_row = !t.header_row,
                }
            }
            // The grown rows get data, as typing under a table does.
            let r = 1 + rng.below(12) as u32;
            num(&mut wb, 0, CellRef::new(r, 9), 4.0);
            changed.push((0, CellRef::new(r, 9)));
            what_text.push_str("table");
        } else {
            let (s, at) = random_cell(&mut rng);
            let text = format!("={}", rng.pick(&["Rate", "Cst", "Nest", "SUM(Rng)", "SUM(T1[Qty])"]));
            formula(&mut wb, s, at, &text);
            changed.push((s, at));
            what_text.push_str("formula2");
        }
        let defs_edit = (66..92).contains(&what);
        if defs_edit && rng.below(2) == 0 {
            // As the engine does: a name or table edit recalculates everything.
            calc.recalc_all(&mut wb);
        } else {
            if changed.is_empty() {
                changed.push(random_cell(&mut rng));
            }
            calc.cells_changed(&mut wb, &changed);
        }
        let mut fresh = wb.clone();
        Calc::new().recalc_all(&mut fresh);
        let (inc, full) = (snapshot(&wb), snapshot(&fresh));
        let _ = writeln!(dump, "{seed} {step} {what_text} {:016x}", hash(&inc));
        if inc != full {
            let diff: Vec<String> =
                inc.lines().zip(full.lines()).filter(|(a, b)| a != b).take(6).map(|(a, b)| format!("{a}  vs fresh  {b}")).collect();
            panic!("seed {seed} step {step} ({what_text}): incremental differs from full recalculation:\n{}", diff.join("\n"));
        }
    }
}

#[test]
fn incremental_matches_full_recalculation() {
    let mut dump = String::new();
    for seed in 1..=40 {
        run(seed, 120, &mut dump);
    }
    if let Some(path) = std::env::var_os("GRIDCRAFT_ORACLE_DUMP") {
        std::fs::write(path, dump).unwrap();
    }
}
