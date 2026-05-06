//! MPS file parser (free-format).
//!
//! Parses .mps and .mps.gz files into an Instance. MPS is line-based
//! (not token-based like LP), with section headers starting in column 1.

use crate::rational::{rat_from_float, rat_from_int, Rat};
use crate::types::{Constraint, Instance, ObjSense, Sense, VarIndex, Variable};
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use num_bigint::BigInt;
use num_traits::Zero;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// Parse an .mps or .mps.gz file into an Instance.
pub fn parse_mps(path: &Path) -> Result<Instance> {
    let text = read_mps_text(path)?;
    parse_mps_string(&text, path)
}

/// Read file contents, decompressing if .gz.
fn read_mps_text(path: &Path) -> Result<String> {
    let path_str = path.to_string_lossy();
    if path_str.ends_with(".gz") {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        let mut decoder = GzDecoder::new(file);
        let mut text = String::new();
        decoder
            .read_to_string(&mut text)
            .with_context(|| format!("Failed to decompress {}", path.display()))?;
        Ok(text)
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))
    }
}

/// MPS section types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Name,
    ObjSense,
    Rows,
    Columns,
    Rhs,
    Ranges,
    Bounds,
}

/// Row type from the ROWS section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowType {
    N, // free (objective)
    E, // equality
    L, // <=
    G, // >=
}

/// Parse MPS text into an Instance.
fn parse_mps_string(text: &str, path: &Path) -> Result<Instance> {
    // Variable registry: name → column index (insertion-ordered).
    let mut var_map: IndexMap<String, usize> = IndexMap::new();

    // Row registry: name → (RowType, row_index). Insertion order = row order.
    let mut row_map: IndexMap<String, (RowType, usize)> = IndexMap::new();

    // Objective row name.
    let mut obj_row: Option<String> = None;

    // Objective sense (default: minimize).
    let mut obj_sense = ObjSense::Minimize;

    // Sparse coefficient matrix: row_name → (var_index → coefficient).
    let mut coefficients: IndexMap<String, BTreeMap<usize, Rat>> = IndexMap::new();

    // RHS values: row_name → value.
    let mut rhs_values: IndexMap<String, Rat> = IndexMap::new();

    // Range values: row_name → value.
    let mut range_values: IndexMap<String, Rat> = IndexMap::new();

    // Integer marker tracking.
    let mut in_integer_block = false;
    let mut integer_vars: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Bounds: var_index → (Option<lb>, Option<ub>). None = infinite.
    let mut explicit_bounds: IndexMap<usize, (Option<Rat>, Option<Rat>)> = IndexMap::new();
    // Track which vars have been marked integer by BV/LI/UI bound types.
    let mut bound_integer_vars: std::collections::HashSet<usize> =
        std::collections::HashSet::new();

    let mut current_section: Option<Section> = None;

    for (line_no, line) in text.lines().enumerate() {
        let line_no = line_no + 1; // 1-based

        // Skip comments (lines starting with $).
        if line.starts_with('$') || line.starts_with('*') {
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Section headers start in column 1 (no leading whitespace).
        if !line.starts_with(' ') && !line.starts_with('\t') {
            let upper = trimmed.to_uppercase();
            if upper.starts_with("NAME") {
                current_section = Some(Section::Name);
                continue;
            } else if upper == "OBJSENSE" {
                current_section = Some(Section::ObjSense);
                continue;
            } else if upper == "ROWS" {
                current_section = Some(Section::Rows);
                continue;
            } else if upper == "COLUMNS" {
                current_section = Some(Section::Columns);
                continue;
            } else if upper == "RHS" {
                current_section = Some(Section::Rhs);
                continue;
            } else if upper == "RANGES" {
                current_section = Some(Section::Ranges);
                continue;
            } else if upper == "BOUNDS" {
                current_section = Some(Section::Bounds);
                continue;
            } else if upper == "ENDATA" {
                break;
            }
            // Unknown section header — skip.
            current_section = None;
            continue;
        }

        // Data lines (indented).
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.is_empty() {
            continue;
        }

        match current_section {
            Some(Section::ObjSense) => {
                let upper = fields[0].to_uppercase();
                if upper == "MAX" || upper == "MAXIMIZE" {
                    obj_sense = ObjSense::Maximize;
                } else if upper == "MIN" || upper == "MINIMIZE" {
                    obj_sense = ObjSense::Minimize;
                }
            }

            Some(Section::Rows) => {
                if fields.len() < 2 {
                    bail!(
                        "{}:{}: ROWS line needs type and name, got: '{}'",
                        path.display(),
                        line_no,
                        trimmed
                    );
                }
                let row_type = match fields[0].to_uppercase().as_str() {
                    "N" => RowType::N,
                    "E" => RowType::E,
                    "L" => RowType::L,
                    "G" => RowType::G,
                    other => bail!(
                        "{}:{}: unknown row type '{}' in ROWS",
                        path.display(),
                        line_no,
                        other
                    ),
                };
                let row_name = fields[1].to_string();

                // First N row is the objective.
                if row_type == RowType::N && obj_row.is_none() {
                    obj_row = Some(row_name.clone());
                }

                let idx = row_map.len();
                row_map.insert(row_name.clone(), (row_type, idx));
                coefficients.entry(row_name).or_default();
            }

            Some(Section::Columns) => {
                // Check for integer markers.
                if fields.len() >= 3 && fields[1].trim_matches('\'') == "MARKER" {
                    let marker_type = fields[2].trim_matches('\'').to_uppercase();
                    if marker_type == "INTORG" {
                        in_integer_block = true;
                    } else if marker_type == "INTEND" {
                        in_integer_block = false;
                    }
                    continue;
                }

                // Format: var_name row_name coeff [row_name coeff]
                if fields.len() < 3 {
                    bail!(
                        "{}:{}: COLUMNS line needs var, row, coeff, got: '{}'",
                        path.display(),
                        line_no,
                        trimmed
                    );
                }

                let var_name = fields[0];
                let var_idx = register_var(var_name, &mut var_map);

                if in_integer_block {
                    integer_vars.insert(var_idx);
                }

                // Process pairs: (row_name, coeff)
                let mut i = 1;
                while i + 1 < fields.len() {
                    let row_name = fields[i];
                    let coeff = parse_mps_number(fields[i + 1]).with_context(|| {
                        format!(
                            "{}:{}: bad coefficient '{}' for var '{}' row '{}'",
                            path.display(),
                            line_no,
                            fields[i + 1],
                            var_name,
                            row_name
                        )
                    })?;

                    if let Some(row_coeffs) = coefficients.get_mut(row_name) {
                        let entry = row_coeffs.entry(var_idx).or_insert_with(Rat::zero);
                        *entry = &*entry + &coeff;
                    }
                    // Ignore references to unknown rows (shouldn't happen in valid MPS).

                    i += 2;
                }
            }

            Some(Section::Rhs) => {
                // Format: rhs_name row_name value [row_name value]
                if fields.len() < 3 {
                    continue; // Skip malformed RHS lines.
                }

                let mut i = 1;
                while i + 1 < fields.len() {
                    let row_name = fields[i].to_string();
                    let value = parse_mps_number(fields[i + 1]).with_context(|| {
                        format!(
                            "{}:{}: bad RHS value '{}' for row '{}'",
                            path.display(),
                            line_no,
                            fields[i + 1],
                            row_name
                        )
                    })?;
                    rhs_values.insert(row_name, value);
                    i += 2;
                }
            }

            Some(Section::Ranges) => {
                // Format: rng_name row_name value [row_name value]
                if fields.len() < 3 {
                    continue;
                }

                let mut i = 1;
                while i + 1 < fields.len() {
                    let row_name = fields[i].to_string();
                    let value = parse_mps_number(fields[i + 1]).with_context(|| {
                        format!(
                            "{}:{}: bad RANGES value '{}' for row '{}'",
                            path.display(),
                            line_no,
                            fields[i + 1],
                            row_name
                        )
                    })?;
                    range_values.insert(row_name, value);
                    i += 2;
                }
            }

            Some(Section::Bounds) => {
                // Format: type bnd_name var_name [value]
                if fields.len() < 3 {
                    bail!(
                        "{}:{}: BOUNDS line needs type, name, var, got: '{}'",
                        path.display(),
                        line_no,
                        trimmed
                    );
                }

                let bound_type = fields[0].to_uppercase();
                // fields[1] is the bound set name (ignored).
                let var_name = fields[2];
                let var_idx = register_var(var_name, &mut var_map);

                // Get or create bound entry; default: lb=0, ub=+inf.
                let entry = explicit_bounds
                    .entry(var_idx)
                    .or_insert((Some(rat_from_int(0)), None));

                match bound_type.as_str() {
                    "LO" => {
                        let val = parse_bound_field(fields.get(3), path, line_no, "LO")?;
                        entry.0 = Some(val);
                    }
                    "UP" => {
                        let val = parse_bound_field(fields.get(3), path, line_no, "UP")?;
                        entry.1 = Some(val);
                    }
                    "FX" => {
                        let val = parse_bound_field(fields.get(3), path, line_no, "FX")?;
                        entry.0 = Some(val.clone());
                        entry.1 = Some(val);
                    }
                    "FR" => {
                        // Free: -inf..+inf.
                        entry.0 = None;
                        entry.1 = None;
                    }
                    "MI" => {
                        // Lower bound = -inf.
                        entry.0 = None;
                    }
                    "PL" => {
                        // Upper bound = +inf (default).
                        entry.1 = None;
                    }
                    "BV" => {
                        // Binary: 0..1 and integer.
                        entry.0 = Some(rat_from_int(0));
                        entry.1 = Some(rat_from_int(1));
                        bound_integer_vars.insert(var_idx);
                    }
                    "LI" => {
                        // Integer lower bound.
                        let val = parse_bound_field(fields.get(3), path, line_no, "LI")?;
                        entry.0 = Some(val);
                        bound_integer_vars.insert(var_idx);
                    }
                    "UI" => {
                        // Integer upper bound.
                        let val = parse_bound_field(fields.get(3), path, line_no, "UI")?;
                        entry.1 = Some(val);
                        bound_integer_vars.insert(var_idx);
                    }
                    other => {
                        bail!(
                            "{}:{}: unknown bound type '{}'",
                            path.display(),
                            line_no,
                            other
                        );
                    }
                }
            }

            Some(Section::Name) | None => {
                // Ignore NAME data lines and lines outside recognized sections.
            }
        }
    }

    // --- Assembly ---

    // Build variables in insertion order.
    let variables: Vec<Variable> = var_map
        .iter()
        .map(|(name, &idx)| {
            let is_integer = integer_vars.contains(&idx) || bound_integer_vars.contains(&idx);

            // Default bounds: lb=0, ub=+inf.
            let (lb, ub) = if let Some((elb, eub)) = explicit_bounds.get(&idx) {
                (elb.clone(), eub.clone())
            } else {
                (Some(rat_from_int(0)), None)
            };

            Variable {
                name: name.clone(),
                is_integer,
                lb,
                ub,
            }
        })
        .collect();

    // Build objective terms from the N row.
    let mut obj_terms: BTreeMap<VarIndex, Rat> = BTreeMap::new();
    if let Some(ref obj_name) = obj_row {
        if let Some(row_coeffs) = coefficients.get(obj_name) {
            for (&col_idx, coeff) in row_coeffs {
                if !coeff.is_zero() {
                    obj_terms.insert(VarIndex(col_idx), coeff.clone());
                }
            }
        }
    }

    // Build constraints from non-N rows.
    let mut constraints = Vec::new();
    for (row_name, &(row_type, _row_idx)) in &row_map {
        if row_type == RowType::N {
            continue;
        }

        let sense = match row_type {
            RowType::E => Sense::Eq,
            RowType::L => Sense::Le,
            RowType::G => Sense::Ge,
            RowType::N => unreachable!(),
        };

        let rhs = rhs_values
            .get(row_name)
            .cloned()
            .unwrap_or_else(Rat::zero);

        let mut terms = BTreeMap::new();
        if let Some(row_coeffs) = coefficients.get(row_name) {
            for (&col_idx, coeff) in row_coeffs {
                if !coeff.is_zero() {
                    terms.insert(VarIndex(col_idx), coeff.clone());
                }
            }
        }

        // Handle RANGES: a ranged row becomes two constraints.
        if let Some(range_val) = range_values.get(row_name) {
            if !range_val.is_zero() {
                // For RANGES, the interpretation depends on the row type and sign of range.
                // Standard interpretation (IBM convention):
                //   G row: rhs <= expr <= rhs + |range|
                //   L row: rhs - |range| <= expr <= rhs
                //   E row: if range >= 0: rhs <= expr <= rhs + range
                //          if range < 0: rhs + range <= expr <= rhs
                let (lower_rhs, upper_rhs) = match row_type {
                    RowType::G => {
                        use num_traits::Signed;
                        (rhs.clone(), &rhs + range_val.abs())
                    }
                    RowType::L => {
                        use num_traits::Signed;
                        (&rhs - range_val.abs(), rhs.clone())
                    }
                    RowType::E => {
                        use num_traits::Signed;
                        if range_val.is_positive() {
                            (rhs.clone(), &rhs + range_val)
                        } else {
                            (&rhs + range_val, rhs.clone())
                        }
                    }
                    RowType::N => unreachable!(),
                };

                // Emit as two constraints: >= lower and <= upper.
                constraints.push(Constraint {
                    name: format!("{}_lo", row_name),
                    sense: Sense::Ge,
                    rhs: lower_rhs,
                    terms: terms.clone(),
                });
                constraints.push(Constraint {
                    name: format!("{}_hi", row_name),
                    sense: Sense::Le,
                    rhs: upper_rhs,
                    terms,
                });
                continue;
            }
        }

        constraints.push(Constraint {
            name: row_name.clone(),
            sense,
            rhs,
            terms,
        });
    }

    Ok(Instance {
        variables,
        obj_sense,
        obj_terms,
        constraints,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register a variable name, returning its index.
fn register_var(name: &str, var_map: &mut IndexMap<String, usize>) -> usize {
    let len = var_map.len();
    *var_map.entry(name.to_string()).or_insert(len)
}

/// Parse a number from an MPS field.
/// Handles integers, decimals, and scientific notation.
fn parse_mps_number(s: &str) -> Result<Rat> {
    // Try exact integer first.
    if let Ok(n) = s.parse::<i64>() {
        return Ok(rat_from_int(n));
    }

    // Try exact decimal (no exponent).
    if !s.contains('e') && !s.contains('E') && !s.contains('d') && !s.contains('D') {
        if let Some(dot_pos) = s.find('.') {
            let decimals = s.len() - dot_pos - 1;
            let without_dot: String = s.chars().filter(|&c| c != '.').collect();
            if let Ok(numer) = without_dot.parse::<i64>() {
                let denom = BigInt::from(10).pow(decimals as u32);
                return Ok(Rat::new(BigInt::from(numer), denom));
            }
        }
    }

    // Scientific notation (e, E, d, D) — fall back to float conversion.
    // Replace Fortran-style 'd'/'D' with 'e' for Rust parsing.
    let normalized = s.replace('d', "e").replace('D', "e");
    let f: f64 = normalized
        .parse()
        .with_context(|| format!("Cannot parse MPS number '{s}'"))?;
    Ok(rat_from_float(f, 1_000_000_000))
}

/// Parse the value field from a BOUNDS line.
fn parse_bound_field(
    field: Option<&&str>,
    path: &Path,
    line_no: usize,
    bound_type: &str,
) -> Result<Rat> {
    let s = field.with_context(|| {
        format!(
            "{}:{}: {} bound requires a value",
            path.display(),
            line_no,
            bound_type
        )
    })?;
    parse_mps_number(s).with_context(|| {
        format!(
            "{}:{}: bad {} bound value '{}'",
            path.display(),
            line_no,
            bound_type,
            s
        )
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rational::rat_from_int;

    fn miplib_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/miplib")
    }

    fn lp_instances_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/instances")
    }

    #[test]
    #[ignore] // slow: parses hundreds of MIPLIB files; run with `cargo test -- --ignored`
    fn test_parse_all_miplib() {
        let dir = miplib_dir();
        if !dir.exists() {
            eprintln!("Skipping test_parse_all_miplib: miplib dir not found");
            return;
        }
        let mut count = 0;
        let mut errors = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "gz") {
                match parse_mps(&path) {
                    Ok(inst) => {
                        assert!(
                            !inst.variables.is_empty(),
                            "{}: no variables parsed",
                            path.display()
                        );
                        count += 1;
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", path.display(), e));
                    }
                }
            }
        }
        if !errors.is_empty() {
            panic!(
                "Failed to parse {} MPS files:\n{}",
                errors.len(),
                errors.join("\n")
            );
        }
        assert!(
            count >= 230,
            "Expected at least 230 .mps.gz files, found {count}"
        );
        eprintln!("Successfully parsed {count} MPS files");
    }

    #[test]
    fn test_parse_markshare_4_0() {
        let path = miplib_dir().join("markshare_4_0.mps.gz");
        if !path.exists() {
            eprintln!("Skipping: markshare_4_0.mps.gz not found");
            return;
        }
        let inst = parse_mps(&path).expect("Failed to parse markshare_4_0");

        // 4 continuous + 30 integer = 34 vars, 4 equality constraints.
        assert_eq!(inst.variables.len(), 34);
        assert_eq!(inst.constraints.len(), 4);
        assert_eq!(inst.obj_sense, ObjSense::Minimize);

        // s1..s4 are continuous, x1..x30 are integer.
        assert!(!inst.variables[0].is_integer); // s1
        assert!(!inst.variables[3].is_integer); // s4
        assert!(inst.variables[4].is_integer); // x1
        assert!(inst.variables[33].is_integer); // x30

        // All constraints are equality.
        for c in &inst.constraints {
            assert_eq!(c.sense, Sense::Eq);
        }

        // Objective: min s1 + s2 + s3 + s4 (only s-vars have coefficients).
        assert_eq!(inst.obj_terms.len(), 4);
        for i in 0..4 {
            assert_eq!(inst.obj_terms[&VarIndex(i)], rat_from_int(1));
        }
    }

    #[test]
    fn test_parse_pk1() {
        let path = miplib_dir().join("pk1.mps.gz");
        if !path.exists() {
            eprintln!("Skipping: pk1.mps.gz not found");
            return;
        }
        let inst = parse_mps(&path).expect("Failed to parse pk1");

        // pk1: 86 variables, 45 constraints.
        assert_eq!(inst.variables.len(), 86);
        assert_eq!(inst.constraints.len(), 45);
        assert_eq!(inst.obj_sense, ObjSense::Minimize);
    }

    #[test]
    fn test_parse_mps_number_variants() {
        // Integer.
        assert_eq!(parse_mps_number("42").unwrap(), rat_from_int(42));
        assert_eq!(parse_mps_number("-7").unwrap(), rat_from_int(-7));
        assert_eq!(parse_mps_number("0").unwrap(), rat_from_int(0));

        // Decimal (exact).
        assert_eq!(
            parse_mps_number("2.5").unwrap(),
            Rat::new(BigInt::from(5), BigInt::from(2))
        );
        assert_eq!(
            parse_mps_number("0.25").unwrap(),
            Rat::new(BigInt::from(1), BigInt::from(4))
        );

        // Scientific notation.
        let r = parse_mps_number("1e-05").unwrap();
        // Should be close to 1/100000.
        let expected = Rat::new(BigInt::from(1), BigInt::from(100000));
        assert_eq!(r, expected);
    }

    #[test]
    fn test_tiny_mps_roundtrip() {
        // Hand-crafted MPS equivalent of tiny1.lp:
        //   min x + y  s.t.  x + 2y >= 2,  2x + y >= 2,  x,y binary
        let mps_text = "\
NAME          tiny1
ROWS
 N  obj
 G  c1
 G  c2
COLUMNS
    MARK0001  'MARKER'                 'INTORG'
    x         obj                  1   c1                   1
    x         c2                   2
    y         obj                  1   c1                   2
    y         c2                   1
    MARK0001  'MARKER'                 'INTEND'
RHS
    rhs       c1                   2   c2                   2
BOUNDS
 UP bnd       x                    1
 UP bnd       y                    1
ENDATA
";
        let inst = parse_mps_string(mps_text, Path::new("tiny1.mps"))
            .expect("Failed to parse tiny1 MPS");

        assert_eq!(inst.variables.len(), 2);
        assert_eq!(inst.variables[0].name, "x");
        assert_eq!(inst.variables[1].name, "y");
        assert!(inst.variables[0].is_integer);
        assert!(inst.variables[1].is_integer);

        // Bounds: default lb=0, explicit ub=1.
        assert_eq!(inst.variables[0].lb, Some(rat_from_int(0)));
        assert_eq!(inst.variables[0].ub, Some(rat_from_int(1)));
        assert_eq!(inst.variables[1].lb, Some(rat_from_int(0)));
        assert_eq!(inst.variables[1].ub, Some(rat_from_int(1)));

        // Objective.
        assert_eq!(inst.obj_sense, ObjSense::Minimize);
        assert_eq!(inst.obj_terms.len(), 2);
        assert_eq!(inst.obj_terms[&VarIndex(0)], rat_from_int(1));
        assert_eq!(inst.obj_terms[&VarIndex(1)], rat_from_int(1));

        // Constraints.
        assert_eq!(inst.constraints.len(), 2);
        assert_eq!(inst.constraints[0].sense, Sense::Ge);
        assert_eq!(inst.constraints[0].rhs, rat_from_int(2));
        assert_eq!(inst.constraints[0].terms[&VarIndex(1)], rat_from_int(2)); // 2y
        assert_eq!(inst.constraints[1].terms[&VarIndex(0)], rat_from_int(2)); // 2x
    }

    /// Compare MPS parse vs LP parse for tiny1 (cross-format validation).
    #[test]
    fn test_cross_format_tiny1() {
        let lp_path = lp_instances_dir().join("tiny1.lp");
        if !lp_path.exists() {
            eprintln!("Skipping cross-format test: tiny1.lp not found");
            return;
        }

        let lp_inst = crate::parser::parse_lp(&lp_path).expect("Failed to parse tiny1.lp");

        // Parse MPS equivalent.
        let mps_text = "\
NAME          tiny1
ROWS
 N  obj
 G  c1
 G  c2
COLUMNS
    MARK0001  'MARKER'                 'INTORG'
    x         obj                  1   c1                   1
    x         c2                   2
    y         obj                  1   c1                   2
    y         c2                   1
    MARK0001  'MARKER'                 'INTEND'
RHS
    rhs       c1                   2   c2                   2
BOUNDS
 UP bnd       x                    1
 UP bnd       y                    1
ENDATA
";
        let mps_inst = parse_mps_string(mps_text, Path::new("tiny1.mps"))
            .expect("Failed to parse tiny1 MPS");

        // Compare.
        assert_eq!(lp_inst.variables.len(), mps_inst.variables.len());
        assert_eq!(lp_inst.obj_sense, mps_inst.obj_sense);
        assert_eq!(lp_inst.obj_terms, mps_inst.obj_terms);
        assert_eq!(lp_inst.constraints.len(), mps_inst.constraints.len());

        for (lv, mv) in lp_inst.variables.iter().zip(mps_inst.variables.iter()) {
            assert_eq!(lv.name, mv.name);
            assert_eq!(lv.is_integer, mv.is_integer);
            assert_eq!(lv.lb, mv.lb);
            assert_eq!(lv.ub, mv.ub);
        }

        for (lc, mc) in lp_inst.constraints.iter().zip(mps_inst.constraints.iter()) {
            assert_eq!(lc.sense, mc.sense);
            assert_eq!(lc.rhs, mc.rhs);
            assert_eq!(lc.terms, mc.terms);
        }
    }
}
