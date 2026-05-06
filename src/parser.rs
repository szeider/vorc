//! LP file parser.
//!
//! Parses .lp format files into an Instance. Section-based token collection
//! handles multi-line continuation automatically.

use crate::rational::{rat_from_int, Rat};
use crate::types::{Constraint, Instance, ObjSense, Sense, VarIndex, Variable};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use num_bigint::BigInt;
use num_traits::Zero;
use std::collections::BTreeMap;
use std::path::Path;

/// Parse an .lp file into an Instance.
pub fn parse_lp(path: &Path) -> Result<Instance> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    // Split into sections: collect all tokens per section header.
    let sections = split_sections(&text)?;

    // Variable registry: name → index (insertion-ordered).
    let mut var_map: IndexMap<String, usize> = IndexMap::new();

    // Determine objective sense.
    let obj_sense = if sections.contains_key("maximize") {
        ObjSense::Maximize
    } else if sections.contains_key("minimize") {
        ObjSense::Minimize
    } else {
        bail!("No Minimize or Maximize section found");
    };

    // Parse objective.
    let obj_section_key = if obj_sense == ObjSense::Maximize {
        "maximize"
    } else {
        "minimize"
    };
    let obj_tokens = sections
        .get(obj_section_key)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let obj_terms = parse_objective(obj_tokens, &mut var_map)?;

    // Parse constraints.
    let st_tokens = sections
        .get("subject to")
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let constraints = parse_constraints(st_tokens, &mut var_map)?;

    // Parse bounds.
    let bounds_tokens = sections
        .get("bounds")
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let explicit_bounds = parse_bounds(bounds_tokens, &mut var_map)?;

    // Collect integer/binary variable names.
    let mut integer_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut binary_vars: std::collections::HashSet<String> = std::collections::HashSet::new();

    for section_key in ["binary", "binaries"] {
        if let Some(tokens) = sections.get(section_key) {
            for tok in tokens {
                register_var(tok, &mut var_map);
                binary_vars.insert(tok.clone());
            }
        }
    }

    for section_key in ["general", "generals", "integer"] {
        if let Some(tokens) = sections.get(section_key) {
            for tok in tokens {
                register_var(tok, &mut var_map);
                integer_vars.insert(tok.clone());
            }
        }
    }

    // Build Variable vec.
    let variables: Vec<Variable> = var_map
        .iter()
        .map(|(name, _idx)| {
            let is_binary = binary_vars.contains(name);
            let is_integer = integer_vars.contains(name) || is_binary;

            // Default bounds: binary → [0,1], others → [0, +inf].
            let default_lb = Some(rat_from_int(0));
            let default_ub = if is_binary {
                Some(rat_from_int(1))
            } else {
                None
            };

            // Explicit bounds override defaults.
            // If a variable appears in explicit_bounds at all, its None entries
            // mean "infinite" (e.g., `x free`), NOT "use default".
            let (lb, ub) = if let Some((elb, eub)) = explicit_bounds.get(name) {
                (elb.clone(), eub.clone())
            } else {
                (default_lb, default_ub)
            };

            Variable {
                name: name.clone(),
                is_integer,
                lb,
                ub,
            }
        })
        .collect();

    Ok(Instance {
        variables,
        obj_sense,
        obj_terms,
        constraints,
    })
}

// ---------------------------------------------------------------------------
// Section splitting
// ---------------------------------------------------------------------------

/// Known section headers (lowercase).
const SECTION_HEADERS: &[&str] = &[
    "minimize",
    "maximize",
    "subject to",
    "bounds",
    "binary",
    "binaries",
    "general",
    "generals",
    "integer",
    "end",
];

/// Detect if a trimmed, lowercased line is a section header.
/// Returns the canonical section name if so.
fn detect_section(line: &str) -> Option<&'static str> {
    let lower = line.to_lowercase();
    let trimmed = lower.trim();
    for &hdr in SECTION_HEADERS {
        if trimmed == hdr {
            return Some(hdr);
        }
    }
    // "subject to" also matches "st", "s.t." and "st:"
    if trimmed == "st" || trimmed == "s.t." || trimmed == "st:" {
        return Some("subject to");
    }
    None
}

/// Split file text into sections. Returns map: section_name → tokens.
fn split_sections(text: &str) -> Result<IndexMap<String, Vec<String>>> {
    let mut sections: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut current_section: Option<String> = None;

    for line in text.lines() {
        // Strip comments (\ is LP comment marker).
        let line = if let Some(pos) = line.find('\\') {
            &line[..pos]
        } else {
            line
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check for section header.
        if let Some(hdr) = detect_section(trimmed) {
            if hdr != "end" {
                current_section = Some(hdr.to_string());
                sections.entry(hdr.to_string()).or_default();
            } else {
                current_section = None;
            }
            continue;
        }

        // Collect tokens into current section.
        if let Some(ref sec) = current_section {
            let tokens = tokenize_line(trimmed);
            sections.get_mut(sec).unwrap().extend(tokens);
        }
    }

    Ok(sections)
}

/// Tokenize a line: split on whitespace, but keep operators as separate tokens.
/// Also splits compound tokens like "3x" into ["3", "x"] and "-y" into ["-", "y"].
fn tokenize_line(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    for tok in line.split_whitespace() {
        split_token(tok, &mut result);
    }
    result
}

/// Split a single whitespace-delimited token into sub-tokens.
/// Handles cases like "3x" → ["3", "x"], "-y" → ["-", "y"],
/// "+2.5x" → ["+", "2.5", "x"], "x:" → label detection.
fn split_token(tok: &str, out: &mut Vec<String>) {
    if tok.is_empty() {
        return;
    }

    // Operator-only tokens.
    if tok == "+" || tok == "-" || tok == "<=" || tok == ">=" || tok == "=" {
        out.push(tok.to_string());
        return;
    }

    // Label token (ends with `:` and no other special chars).
    if tok.ends_with(':') && !tok.contains('<') && !tok.contains('>') && !tok.contains('=') {
        out.push(tok.to_string());
        return;
    }

    let bytes = tok.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;

        // Leading +/- → emit as operator, unless part of a number like "+2.5x".
        if (ch == '+' || ch == '-') && i == 0 {
            // Check if followed by a digit or dot → it's a sign for a number.
            if i + 1 < bytes.len()
                && (bytes[i + 1].is_ascii_digit() || bytes[i + 1] == b'.')
            {
                // Part of a number — fall through to number parsing below.
            } else {
                out.push(ch.to_string());
                i += 1;
                continue;
            }
        }

        // Number (digits, dots, leading sign, e-notation).
        if ch.is_ascii_digit() || ch == '.' || ((ch == '+' || ch == '-') && i == 0) {
            let start = i;
            if ch == '+' || ch == '-' {
                i += 1;
            }
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || bytes[i] == b'.'
                    || bytes[i] == b'e'
                    || bytes[i] == b'E'
                    || ((bytes[i] == b'+' || bytes[i] == b'-')
                        && i > 0
                        && (bytes[i - 1] == b'e' || bytes[i - 1] == b'E')))
            {
                i += 1;
            }
            out.push(tok[start..i].to_string());
            continue;
        }

        // Comparison operators.
        if ch == '<' || ch == '>' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                out.push(tok[i..i + 2].to_string());
                i += 2;
            } else {
                out.push(ch.to_string());
                i += 1;
            }
            continue;
        }

        if ch == '=' {
            out.push("=".to_string());
            i += 1;
            continue;
        }

        // Variable name (alpha, digit, underscore).
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            // Check for trailing colon (label).
            if i < bytes.len() && bytes[i] == b':' {
                i += 1;
                out.push(tok[start..i].to_string());
            } else {
                out.push(tok[start..i].to_string());
            }
            continue;
        }

        // Skip unrecognized characters.
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Variable registry
// ---------------------------------------------------------------------------

/// Register a variable name, returning its VarIndex.
fn register_var(name: &str, var_map: &mut IndexMap<String, usize>) -> VarIndex {
    let len = var_map.len();
    let idx = *var_map.entry(name.to_string()).or_insert(len);
    VarIndex(idx)
}

// ---------------------------------------------------------------------------
// Number parsing
// ---------------------------------------------------------------------------

/// Parse a numeric string to Rat.
/// Handles integers ("3"), decimals ("2.5"), and fractions ("1/3").
fn parse_number(s: &str) -> Result<Rat> {
    // Fraction notation.
    if let Some(pos) = s.find('/') {
        let numer: i64 = s[..pos]
            .parse()
            .with_context(|| format!("Bad fraction numerator in '{s}'"))?;
        let denom: i64 = s[pos + 1..]
            .parse()
            .with_context(|| format!("Bad fraction denominator in '{s}'"))?;
        return Ok(Rat::new(BigInt::from(numer), BigInt::from(denom)));
    }

    // Decimal notation.
    if let Some(dot_pos) = s.find('.') {
        let decimals = s.len() - dot_pos - 1;
        let without_dot: String = s.chars().filter(|&c| c != '.').collect();
        let numer: i64 = without_dot
            .parse()
            .with_context(|| format!("Bad decimal number '{s}'"))?;
        let denom = BigInt::from(10).pow(decimals as u32);
        return Ok(Rat::new(BigInt::from(numer), denom));
    }

    // Plain integer.
    let n: i64 = s
        .parse()
        .with_context(|| format!("Bad integer '{s}'"))?;
    Ok(rat_from_int(n))
}

/// Check if a string looks like a number (starts with digit, dot, or sign-digit).
fn looks_like_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.as_bytes()[0];
    if first.is_ascii_digit() || first == b'.' {
        return true;
    }
    if (first == b'+' || first == b'-') && s.len() > 1 {
        let second = s.as_bytes()[1];
        return second.is_ascii_digit() || second == b'.';
    }
    false
}

/// Check if a token is a sense operator.
fn is_sense(s: &str) -> Option<Sense> {
    match s {
        ">=" => Some(Sense::Ge),
        "<=" => Some(Sense::Le),
        "=" => Some(Sense::Eq),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Linear expression parsing
// ---------------------------------------------------------------------------

/// Parse a linear expression from tokens, returning (terms, next_position).
/// Stops at a sense operator (<=, >=, =) or end of tokens.
fn parse_linear_expr(
    tokens: &[String],
    start: usize,
    var_map: &mut IndexMap<String, usize>,
) -> Result<(BTreeMap<VarIndex, Rat>, usize)> {
    let mut terms = BTreeMap::new();
    let mut pos = start;
    let mut pending_sign: Rat = rat_from_int(1);
    let mut pending_coeff: Option<Rat> = None;

    while pos < tokens.len() {
        let tok = &tokens[pos];

        // Sense operator → stop.
        if is_sense(tok).is_some() {
            break;
        }

        if tok == "+" {
            // If there's a pending coefficient with no variable, something's off — ignore.
            pending_sign = rat_from_int(1);
            pending_coeff = None;
            pos += 1;
            continue;
        }

        if tok == "-" {
            pending_sign = rat_from_int(-1);
            pending_coeff = None;
            pos += 1;
            continue;
        }

        if looks_like_number(tok) {
            let num = parse_number(tok)?;
            pending_coeff = Some(&pending_sign * &num);
            pos += 1;
            continue;
        }

        // Must be a variable name.
        let coeff = pending_coeff.take().unwrap_or(pending_sign.clone());
        let var_idx = register_var(tok, var_map);
        // Accumulate (handles duplicate variable entries).
        let entry = terms.entry(var_idx).or_insert_with(Rat::zero);
        *entry = &*entry + &coeff;
        pending_sign = rat_from_int(1);
        pos += 1;
    }

    Ok((terms, pos))
}

// ---------------------------------------------------------------------------
// Objective parsing
// ---------------------------------------------------------------------------

/// Parse the objective section tokens.
fn parse_objective(
    tokens: &[String],
    var_map: &mut IndexMap<String, usize>,
) -> Result<BTreeMap<VarIndex, Rat>> {
    if tokens.is_empty() {
        return Ok(BTreeMap::new());
    }

    // Skip label token (ends with ':').
    let start = if tokens[0].ends_with(':') { 1 } else { 0 };

    let (terms, _) = parse_linear_expr(tokens, start, var_map)?;
    Ok(terms)
}

// ---------------------------------------------------------------------------
// Constraint parsing
// ---------------------------------------------------------------------------

/// Parse the Subject To section tokens into Constraints.
fn parse_constraints(
    tokens: &[String],
    var_map: &mut IndexMap<String, usize>,
) -> Result<Vec<Constraint>> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut constraints = Vec::new();
    let mut pos = 0;
    let mut unnamed_counter = 0;

    while pos < tokens.len() {
        // Check for label.
        let name = if tokens[pos].ends_with(':') {
            let label = tokens[pos].trim_end_matches(':').to_string();
            pos += 1;
            label
        } else {
            unnamed_counter += 1;
            format!("_c{unnamed_counter}")
        };

        // Parse LHS linear expression.
        let (terms, next_pos) = parse_linear_expr(tokens, pos, var_map)?;
        pos = next_pos;

        // Parse sense.
        if pos >= tokens.len() {
            bail!("Constraint '{name}' missing sense operator");
        }
        let sense = is_sense(&tokens[pos])
            .with_context(|| format!("Expected sense operator, got '{}' in constraint '{name}'", tokens[pos]))?;
        pos += 1;

        // Parse RHS (could be a linear expression, but typically just a number).
        // For simplicity, handle: number, or -number, or a full expression.
        if pos >= tokens.len() {
            bail!("Constraint '{name}' missing RHS");
        }

        let rhs = if looks_like_number(&tokens[pos]) {
            let r = parse_number(&tokens[pos])?;
            pos += 1;
            r
        } else if tokens[pos] == "-" && pos + 1 < tokens.len() && looks_like_number(&tokens[pos + 1]) {
            let r = parse_number(&tokens[pos + 1])?;
            pos += 2;
            -r
        } else {
            bail!("Expected numeric RHS for constraint '{name}', got '{}'", tokens[pos]);
        };

        constraints.push(Constraint {
            name,
            sense,
            rhs,
            terms,
        });
    }

    Ok(constraints)
}

// ---------------------------------------------------------------------------
// Bounds parsing
// ---------------------------------------------------------------------------

/// Parse the Bounds section tokens.
/// Returns map: var_name → (Option<lb>, Option<ub>).
/// Handles: `lb <= var <= ub`, `var >= lb`, `var <= ub`, `var = val`, `var free`.
fn parse_bounds(
    tokens: &[String],
    var_map: &mut IndexMap<String, usize>,
) -> Result<IndexMap<String, (Option<Rat>, Option<Rat>)>> {
    let mut bounds: IndexMap<String, (Option<Rat>, Option<Rat>)> = IndexMap::new();

    // Re-tokenize by line would be ideal, but we have a flat token stream.
    // Strategy: scan for patterns in the token stream.
    let mut pos = 0;

    while pos < tokens.len() {
        // Pattern: "var free"
        if pos + 1 < tokens.len()
            && !looks_like_number(&tokens[pos])
            && tokens[pos] != "-"
            && tokens[pos + 1].to_lowercase() == "free"
        {
            let name = tokens[pos].clone();
            register_var(&name, var_map);
            bounds.insert(name, (None, None));
            pos += 2;
            continue;
        }

        // Pattern: number sense var [sense number]
        // e.g., "0 <= x <= 4" or "-inf <= x <= inf"
        if looks_like_number(&tokens[pos]) || is_inf_token(&tokens[pos]) || tokens[pos] == "-" {
            // Parse left value.
            let (left_val, next) = parse_bound_value(tokens, pos)?;
            pos = next;

            // Expect sense.
            if pos >= tokens.len() {
                break;
            }
            let sense1 = is_sense(&tokens[pos]);
            if sense1.is_none() {
                pos += 1;
                continue;
            }
            let sense1 = sense1.unwrap();
            pos += 1;

            // Expect variable name.
            if pos >= tokens.len() {
                break;
            }
            let var_name = tokens[pos].clone();
            register_var(&var_name, var_map);
            pos += 1;

            let mut lb = None;
            let mut ub = None;

            match sense1 {
                Sense::Le => {
                    // left <= var
                    if let Some(v) = left_val {
                        lb = Some(v);
                    }
                }
                Sense::Ge => {
                    // left >= var → var <= left
                    if let Some(v) = left_val {
                        ub = Some(v);
                    }
                }
                Sense::Eq => {
                    if let Some(v) = left_val {
                        lb = Some(v.clone());
                        ub = Some(v);
                    }
                }
            }

            // Check for second sense+value: "<= 4"
            if pos + 1 < tokens.len()
                && let Some(sense2) = is_sense(&tokens[pos])
            {
                pos += 1;
                let (right_val, next) = parse_bound_value(tokens, pos)?;
                pos = next;
                match sense2 {
                    Sense::Le => {
                        // var <= right
                        if let Some(v) = right_val {
                            ub = Some(v);
                        }
                    }
                    Sense::Ge => {
                        // var >= right
                        if let Some(v) = right_val {
                            lb = Some(v);
                        }
                    }
                    Sense::Eq => {
                        if let Some(v) = right_val {
                            lb = Some(v.clone());
                            ub = Some(v);
                        }
                    }
                }
            }

            bounds.insert(var_name, (lb, ub));
            continue;
        }

        // Pattern: var sense number
        // e.g., "x <= 4", "x >= 0", "x = 5"
        if !looks_like_number(&tokens[pos])
            && tokens[pos] != "-"
            && !is_inf_token(&tokens[pos])
        {
            let var_name = tokens[pos].clone();
            register_var(&var_name, var_map);
            pos += 1;

            if pos >= tokens.len() {
                break;
            }
            if let Some(sense) = is_sense(&tokens[pos]) {
                pos += 1;
                let (val, next) = parse_bound_value(tokens, pos)?;
                pos = next;

                let (lb, ub) = match sense {
                    Sense::Le => (Some(rat_from_int(0)), val),
                    Sense::Ge => (val, None),
                    Sense::Eq => (val.clone(), val),
                };
                bounds.insert(var_name, (lb, ub));
                continue;
            }
        }

        pos += 1;
    }

    Ok(bounds)
}

/// Check if a token represents infinity.
fn is_inf_token(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower == "inf" || lower == "infinity" || lower == "+inf" || lower == "+infinity"
}

fn is_neg_inf_token(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower == "-inf" || lower == "-infinity"
}

/// Parse a bound value (number or ±inf) from the token stream.
/// Returns (Option<Rat>, next_pos). None means infinite.
fn parse_bound_value(tokens: &[String], pos: usize) -> Result<(Option<Rat>, usize)> {
    if pos >= tokens.len() {
        bail!("Expected bound value at end of tokens");
    }

    // -inf
    if is_neg_inf_token(&tokens[pos]) {
        return Ok((None, pos + 1));
    }
    // - inf (two tokens)
    if tokens[pos] == "-" && pos + 1 < tokens.len() && is_inf_token(&tokens[pos + 1]) {
        return Ok((None, pos + 2));
    }
    // +inf or inf
    if is_inf_token(&tokens[pos]) {
        return Ok((None, pos + 1));
    }

    // Negative number: "- 3" (two tokens)
    if tokens[pos] == "-" && pos + 1 < tokens.len() && looks_like_number(&tokens[pos + 1]) {
        let val = parse_number(&tokens[pos + 1])?;
        return Ok((Some(-val), pos + 2));
    }

    // Regular number.
    if looks_like_number(&tokens[pos]) {
        let val = parse_number(&tokens[pos])?;
        return Ok((Some(val), pos + 1));
    }

    bail!("Expected bound value, got '{}'", tokens[pos]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rational::rat;

    fn test_instances_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/instances")
    }

    #[test]
    fn test_parse_tiny1() {
        let path = test_instances_dir().join("tiny1.lp");
        let inst = parse_lp(&path).expect("Failed to parse tiny1.lp");

        assert_eq!(inst.variables.len(), 2);
        assert_eq!(inst.variables[0].name, "x");
        assert_eq!(inst.variables[1].name, "y");
        assert!(inst.variables[0].is_integer);
        assert!(inst.variables[1].is_integer);

        // Objective: min x + y
        assert_eq!(inst.obj_sense, ObjSense::Minimize);
        assert_eq!(inst.obj_terms.len(), 2);
        assert_eq!(inst.obj_terms[&VarIndex(0)], rat_from_int(1));
        assert_eq!(inst.obj_terms[&VarIndex(1)], rat_from_int(1));

        // Two constraints.
        assert_eq!(inst.constraints.len(), 2);
        // c1: x + 2y >= 2
        assert_eq!(inst.constraints[0].sense, Sense::Ge);
        assert_eq!(inst.constraints[0].rhs, rat_from_int(2));
        assert_eq!(inst.constraints[0].terms[&VarIndex(1)], rat_from_int(2));
        // c2: 2x + y >= 2
        assert_eq!(inst.constraints[1].terms[&VarIndex(0)], rat_from_int(2));

        // Binary bounds: [0, 1].
        assert_eq!(inst.variables[0].lb, Some(rat_from_int(0)));
        assert_eq!(inst.variables[0].ub, Some(rat_from_int(1)));
    }

    #[test]
    fn test_parse_tiny2() {
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).expect("Failed to parse tiny2.lp");

        assert_eq!(inst.variables.len(), 2);
        assert_eq!(inst.obj_sense, ObjSense::Maximize);
        // obj: 3x + 4y
        assert_eq!(inst.obj_terms[&VarIndex(0)], rat_from_int(3));
        assert_eq!(inst.obj_terms[&VarIndex(1)], rat_from_int(4));

        // c1: 2x + 3y <= 4
        assert_eq!(inst.constraints.len(), 1);
        assert_eq!(inst.constraints[0].sense, Sense::Le);
        assert_eq!(inst.constraints[0].rhs, rat_from_int(4));
    }

    #[test]
    fn test_parse_genint4() {
        let path = test_instances_dir().join("genint4.lp");
        let inst = parse_lp(&path).expect("Failed to parse genint4.lp");

        assert_eq!(inst.variables.len(), 4);
        assert!(inst.variables.iter().all(|v| v.is_integer));

        // Explicit bounds: 0 <= x1 <= 4, etc.
        assert_eq!(inst.variables[0].lb, Some(rat_from_int(0)));
        assert_eq!(inst.variables[0].ub, Some(rat_from_int(4)));
        assert_eq!(inst.variables[1].ub, Some(rat_from_int(3)));
        assert_eq!(inst.variables[2].ub, Some(rat_from_int(3)));
        assert_eq!(inst.variables[3].ub, Some(rat_from_int(3)));
    }

    #[test]
    fn test_parse_mix3() {
        let path = test_instances_dir().join("mix3.lp");
        let inst = parse_lp(&path).expect("Failed to parse mix3.lp");

        assert_eq!(inst.variables.len(), 3);
        // x: general integer, y: binary, z: continuous
        assert!(inst.variables[0].is_integer); // x
        assert!(inst.variables[1].is_integer); // y (binary)
        assert!(!inst.variables[2].is_integer); // z

        // z has default bounds [0, +inf]
        assert_eq!(inst.variables[2].lb, Some(rat_from_int(0)));
        assert_eq!(inst.variables[2].ub, None);
    }

    #[test]
    fn test_parse_all_instances() {
        let dir = test_instances_dir();
        if !dir.exists() {
            eprintln!("Skipping test_parse_all_instances: test dir not found");
            return;
        }
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "lp") {
                let result = parse_lp(&path);
                assert!(
                    result.is_ok(),
                    "Failed to parse {}: {}",
                    path.display(),
                    result.unwrap_err()
                );
                let inst = result.unwrap();
                assert!(
                    !inst.variables.is_empty(),
                    "{}: no variables parsed",
                    path.display()
                );
                count += 1;
            }
        }
        assert!(count >= 30, "Expected at least 30 .lp files, found {count}");
    }

    #[test]
    fn test_parse_number_variants() {
        assert_eq!(parse_number("3").unwrap(), rat_from_int(3));
        assert_eq!(parse_number("2.5").unwrap(), rat(5, 2));
        assert_eq!(parse_number("0.25").unwrap(), rat(1, 4));
        assert_eq!(parse_number("-7").unwrap(), rat_from_int(-7));
    }

    #[test]
    fn test_tokenize_compound() {
        let tokens = tokenize_line("3x + 2.5y - z");
        assert_eq!(tokens, vec!["3", "x", "+", "2.5", "y", "-", "z"]);
    }

    #[test]
    fn test_tokenize_no_space() {
        let tokens = tokenize_line("-y");
        assert_eq!(tokens, vec!["-", "y"]);
    }
}
