//! Gomory cut generation at the LP root.
//!
//! Uses GRBBinvRowi to extract tableau row multipliers for basic
//! fractional integer variables, derives {lin} + {rnd} CG cuts.

use crate::dual::apply_rnd;
use crate::oracle::Oracle;
use crate::rational::{rat_from_float, rat_from_int, Rat};
use crate::types::{
    ConIndex, Constraint, Instance, LpResult, Reason, RunStats, Sense, VarIndex,
};
use crate::vipr::CertificateBuilder;
use anyhow::{bail, Result};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::collections::{BTreeMap, BTreeSet};

const INTEGRALITY_TOL: f64 = 1e-6;
const MAX_LCM_DENOM: u64 = 1_000_000_000_000;
const CASCADE_DENOMS: &[i64] = &[
    1_000_000, 10_000_000, 100_000_000, 1_000_000_000,
    10_000_000_000, 100_000_000_000, 1_000_000_000_000,
];

/// Generate Gomory cuts at the root node for up to `max_rounds` rounds.
pub fn generate_gomory_cuts(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    max_rounds: usize,
    stats: &mut RunStats,
) -> Result<()> {
    let mut total_cuts = 0usize;
    for round in 0..max_rounds {
        let cuts = gomory_round(oracle, cert, instance, bound_map, stats)?;
        if cuts == 0 {
            println!("  Gomory round {}: no cuts found, stopping", round + 1);
            break;
        }
        total_cuts += cuts;
        println!("  Gomory round {}: {} cuts added", round + 1, cuts);
    }
    if total_cuts > 0 {
        println!("Gomory: {} total cuts added", total_cuts);
    }
    Ok(())
}

fn gomory_round(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
) -> Result<usize> {
    // Solve LP and get basis head in one call (while model is relaxed).
    let (lp, bhead) = oracle.solve_lp_with_basis()?;
    let primals = match lp {
        LpResult::Optimal { primals, .. } => primals,
        LpResult::Infeasible { .. } => return Ok(0),
        LpResult::Unbounded => bail!("LP unbounded at root"),
    };

    let n_vars = instance.variables.len();
    let num_con_rows = oracle.num_con_rows();

    // Collect candidates: basic fractional integer variables.
    let mut candidates: Vec<(usize, VarIndex, f64)> = Vec::new();
    for (basis_pos, &bh) in bhead.iter().enumerate() {
        if bh < 0 { continue; }
        let vi = bh as usize;
        if vi >= n_vars || !instance.variables[vi].is_integer { continue; }
        let frac = (primals[vi] - primals[vi].round()).abs();
        if frac > INTEGRALITY_TOL {
            candidates.push((basis_pos, VarIndex(vi), primals[vi]));
        }
    }

    // For each candidate, get B^{-1} row (needs separate LP solve to be in relaxed state).
    // We'll re-solve once per candidate. This is expensive but correct.
    let mut cuts = 0usize;
    for (basis_pos, var_idx, lp_val) in candidates {
        let (_lp2, binv_row) = oracle.solve_lp_with_binv_row(basis_pos)?;
        if binv_row.is_empty() { continue; }

        // Build float multiplier map from B^{-1} row.
        let mut float_mults: BTreeMap<ConIndex, f64> = BTreeMap::new();
        for &(row_idx, val) in &binv_row {
            if val.abs() < 1e-12 { continue; }
            if row_idx < num_con_rows {
                float_mults.insert(ConIndex(row_idx), val);
            }
        }
        if float_mults.is_empty() { continue; }

        match try_cut(&float_mults, cert, instance, bound_map, stats, var_idx, lp_val, oracle) {
            Ok(true) => cuts += 1,
            Ok(false) => {}
            Err(e) => eprintln!("    Gomory skip {}: {e:#}", instance.variables[var_idx.0].name),
        }
    }
    Ok(cuts)
}

fn try_cut(
    float_mults: &BTreeMap<ConIndex, f64>,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
    var_idx: VarIndex,
    lp_val: f64,
    oracle: &mut Oracle,
) -> Result<bool> {
    let mut last_err = None;
    for &max_denom in CASCADE_DENOMS {
        let mut rat_mults: BTreeMap<ConIndex, Rat> = BTreeMap::new();
        for (&idx, &val) in float_mults {
            let r = rat_from_float(val, max_denom);
            if !r.is_zero() { rat_mults.insert(idx, r); }
        }
        if rat_mults.is_empty() { continue; }

        match derive_cut(cert, instance, bound_map, stats, rat_mults, var_idx, lp_val, oracle) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) => last_err = Some(e),
        }
    }
    if let Some(e) = last_err { Err(e) } else { Ok(false) }
}

fn derive_cut(
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
    rat_mults: BTreeMap<ConIndex, Rat>,
    var_idx: VarIndex,
    lp_val: f64,
    oracle: &mut Oracle,
) -> Result<bool> {
    let con_constraints = cert.con_constraints();
    let target_sense = match instance.obj_sense {
        crate::types::ObjSense::Minimize => Sense::Ge,
        crate::types::ObjSense::Maximize => Sense::Le,
    };

    // Compute raw linear combination.
    let mut lhs: BTreeMap<VarIndex, Rat> = BTreeMap::new();
    let mut rhs = Rat::zero();
    for (&idx, lambda) in &rat_mults {
        if idx.0 >= con_constraints.len() { continue; }
        let con = &con_constraints[idx.0];
        for (&v, coeff) in &con.terms {
            *lhs.entry(v).or_insert_with(Rat::zero) += lambda * coeff;
        }
        rhs += lambda * &con.rhs;
    }
    lhs.retain(|_, v| !v.is_zero());

    // Zero out continuous variables via bound repair.
    let mut repair_mults = rat_mults.clone();
    for (&v, coeff) in &lhs.clone() {
        if instance.variables[v.0].is_integer || coeff.is_zero() { continue; }
        let deficit = -coeff;
        let (lb_ci, ub_ci) = bound_map.get(&v).copied().unwrap_or((None, None));
        let repair_ci = match target_sense {
            Sense::Ge => if deficit.is_positive() { lb_ci } else { ub_ci },
            Sense::Le => if deficit.is_negative() { lb_ci } else { ub_ci },
            Sense::Eq => unreachable!(),
        };
        let con_idx = match repair_ci {
            Some(idx) => idx,
            None => return Ok(false),
        };
        if con_idx.0 >= con_constraints.len() { return Ok(false); }
        let bound_con = &con_constraints[con_idx.0];
        *repair_mults.entry(con_idx).or_insert_with(Rat::zero) += &deficit;
        for (&bv, bc) in &bound_con.terms {
            *lhs.entry(bv).or_insert_with(Rat::zero) += &deficit * bc;
        }
        rhs += &deficit * &bound_con.rhs;
    }
    lhs.retain(|_, v| !v.is_zero());

    // LCM scaling to integer coefficients.
    let mut lcm_denom = BigInt::one();
    for (&v, coeff) in &lhs {
        if !instance.variables[v.0].is_integer { return Ok(false); }
        lcm_denom = lcm(&lcm_denom, &coeff.denom().clone());
        if lcm_denom > BigInt::from(MAX_LCM_DENOM) { return Ok(false); }
    }

    let scale = BigRational::from_integer(lcm_denom);
    let scaled_mults: BTreeMap<ConIndex, Rat> = repair_mults
        .iter()
        .map(|(&k, v)| (k, v * &scale))
        .filter(|(_, v)| !v.is_zero())
        .collect();

    // Recompute from scaled multipliers.
    let mut final_lhs: BTreeMap<VarIndex, Rat> = BTreeMap::new();
    let mut final_rhs = Rat::zero();
    for (&idx, lambda) in &scaled_mults {
        if idx.0 >= con_constraints.len() { continue; }
        let con = &con_constraints[idx.0];
        for (&v, coeff) in &con.terms {
            *final_lhs.entry(v).or_insert_with(Rat::zero) += lambda * coeff;
        }
        final_rhs += lambda * &con.rhs;
    }
    final_lhs.retain(|_, v| !v.is_zero());

    // Verify integer coefficients.
    for (_, coeff) in &final_lhs {
        if !coeff.is_integer() { return Ok(false); }
    }

    // Check suitability.
    for (&idx, lambda) in &scaled_mults {
        if idx.0 >= con_constraints.len() { return Ok(false); }
        let con = &con_constraints[idx.0];
        let sign = suitability_sign(lambda, con.sense);
        match target_sense {
            Sense::Ge => { if sign < 0 { return Ok(false); } }
            Sense::Le => { if sign > 0 { return Ok(false); } }
            Sense::Eq => unreachable!(),
        }
    }

    // Create {lin} derivation.
    let lin_con = Constraint {
        name: format!("d{}", cert.next_index().0 - cert.con_count()),
        sense: target_sense,
        rhs: final_rhs,
        terms: final_lhs,
    };
    let multipliers: Vec<(ConIndex, Rat)> = scaled_mults.into_iter().collect();
    stats.num_lin_steps += 1;
    let lin_idx = cert.add_derivation(lin_con.clone(), Reason::Lin(multipliers), BTreeSet::new());

    // Apply {rnd}.
    let (cut_idx, cut_con) = if let Some((mut rnd_con, rnd_reason)) = apply_rnd(&lin_con, lin_idx) {
        rnd_con.name = format!("d{}", cert.next_index().0 - cert.con_count());
        stats.num_rnd_steps += 1;
        let rnd_idx = cert.add_derivation(rnd_con.clone(), rnd_reason, BTreeSet::new());
        (rnd_idx, rnd_con)
    } else {
        (lin_idx, lin_con)
    };

    // Add as permanent cut to oracle.
    oracle.add_permanent_cut(&cut_con.terms, cut_con.sense, &cut_con.rhs, cut_idx)?;
    stats.num_gomory_cuts += 1;

    println!(
        "    CG cut for {} (val={:.6}): {} {} {}",
        instance.variables[var_idx.0].name, lp_val,
        format_lhs(&cut_con.terms, instance),
        match cut_con.sense { Sense::Ge => ">=", Sense::Le => "<=", Sense::Eq => "=" },
        crate::vipr::fmt_rational(&cut_con.rhs),
    );
    Ok(true)
}

fn lcm(a: &BigInt, b: &BigInt) -> BigInt {
    if a.is_zero() || b.is_zero() { return BigInt::zero(); }
    let g = gcd(a.abs(), b.abs());
    (a / &g).abs() * b.abs()
}

fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() { let t = &a % &b; a = b; b = t; }
    a
}

fn suitability_sign(lambda: &Rat, sense: Sense) -> i8 {
    match sense {
        Sense::Eq => 0,
        Sense::Ge => if lambda.is_positive() { 1 } else if lambda.is_negative() { -1 } else { 0 },
        Sense::Le => if lambda.is_positive() { -1 } else if lambda.is_negative() { 1 } else { 0 },
    }
}

fn format_lhs(terms: &BTreeMap<VarIndex, Rat>, instance: &Instance) -> String {
    terms.iter()
        .map(|(v, c)| {
            if c == &rat_from_int(1) { instance.variables[v.0].name.clone() }
            else if c == &rat_from_int(-1) { format!("-{}", instance.variables[v.0].name) }
            else { format!("{}*{}", crate::vipr::fmt_rational(c), instance.variables[v.0].name) }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}
