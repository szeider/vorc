//! Bound propagation for tightening variable bounds before B&B.
//!
//! For each constraint `a^T x [sense] b`, for each variable x_j:
//! compute an implied bound using other variables' finite bounds.
//! If tighter than the current bound: derive as `{lin}` step,
//! optionally `{rnd}` for integer vars, and add to the certificate,
//! oracle model, and bound_map. Iterate to fixpoint.

use crate::dual::apply_rnd;
use crate::oracle::Oracle;
use crate::rational::{rat_from_int, Rat};
use crate::types::{
    ConIndex, Constraint, Instance, Reason, RunStats, Sense, VarIndex,
};
use crate::vipr::CertificateBuilder;
use anyhow::Result;
use num_traits::{Signed, Zero};
use std::collections::BTreeMap;

/// Maximum propagation rounds.
const MAX_ROUNDS: usize = 50;

/// Run bound propagation to fixpoint.
///
/// Scans all model constraints (not bounds) and derives implied variable bounds.
/// Each tightened bound is added as a `{lin}` (+ optional `{rnd}`) derivation
/// to the certificate, as a permanent row in the oracle, and updates `bound_map`.
pub fn propagate_bounds(
    cert: &mut CertificateBuilder,
    oracle: &mut Oracle,
    instance: &Instance,
    bound_map: &mut BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
) -> Result<()> {
    let mut total_tightened = 0usize;

    for round in 0..MAX_ROUNDS {
        let tightened = propagation_round(cert, oracle, instance, bound_map, stats)?;
        if tightened == 0 {
            if round > 0 {
                println!("  Propagation fixpoint after {round} rounds, {total_tightened} bounds tightened");
            }
            break;
        }
        total_tightened += tightened;
    }

    if total_tightened > 0 {
        println!("Propagation: {total_tightened} bounds tightened");
    }
    Ok(())
}

/// One round of propagation. Returns number of bounds tightened.
fn propagation_round(
    cert: &mut CertificateBuilder,
    oracle: &mut Oracle,
    instance: &Instance,
    bound_map: &mut BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
) -> Result<usize> {
    let mut tightened = 0usize;
    let con_count = cert.con_count();

    // Model constraints in CON section are after all bound rows.
    let n_bounds = con_count - instance.constraints.len();

    for (model_idx, con) in instance.constraints.iter().enumerate() {
        let con_ci = ConIndex(n_bounds + model_idx);

        // For each variable in this constraint, try to derive an implied bound.
        let var_indices: Vec<VarIndex> = con.terms.keys().copied().collect();
        for &var_j in &var_indices {
            let a_j = &con.terms[&var_j];
            if a_j.is_zero() {
                continue;
            }

            // Try to compute implied upper and lower bounds for var_j.
            if let Some(count) = try_implied_bound(
                cert, oracle, instance, bound_map, stats,
                con, con_ci, var_j, a_j, &var_indices, n_bounds,
            )? {
                tightened += count;
            }
        }
    }

    Ok(tightened)
}

/// Try to derive an implied bound for variable `var_j` from constraint `con`.
///
/// From `a_j * x_j + sum_{i != j} a_i * x_i [sense] b`:
/// - If a_j > 0 and sense is Le: x_j <= (b - sum_i a_i * lb_i) / a_j (ub for x_j)
/// - If a_j > 0 and sense is Ge: x_j >= (b - sum_i a_i * ub_i) / a_j (lb for x_j)
/// - If a_j < 0 and sense is Le: x_j >= (b - sum_i a_i * lb_i) / a_j (lb for x_j, reversed by negative division)
/// - If a_j < 0 and sense is Ge: x_j <= (b - sum_i a_i * ub_i) / a_j (ub for x_j)
/// - Eq constraints: derive both directions.
fn try_implied_bound(
    cert: &mut CertificateBuilder,
    oracle: &mut Oracle,
    instance: &Instance,
    bound_map: &mut BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    stats: &mut RunStats,
    con: &Constraint,
    con_ci: ConIndex,
    var_j: VarIndex,
    a_j: &Rat,
    var_indices: &[VarIndex],
    _n_bounds: usize,
) -> Result<Option<usize>> {
    let mut count = 0;

    let directions: &[Sense] = match con.sense {
        Sense::Le => &[Sense::Le],
        Sense::Ge => &[Sense::Ge],
        Sense::Eq => &[Sense::Le, Sense::Ge],
    };

    for &dir in directions {
        // Compute the sum of a_i * bound_i for all i != j.
        // For Le direction: we want to maximize the sum (use ub for positive a_i, lb for negative a_i).
        //   This gives us the weakest possible remaining bound → tightest implied bound on x_j.
        //   Actually for Le: a_j * x_j <= b - sum, so to get tightest UB on x_j,
        //   we want to MINIMIZE the sum (use lb for positive a_i, ub for negative a_i).
        //
        // Correction: For Le: a_j*x_j + ... <= b → a_j*x_j <= b - sum_{i!=j} a_i*x_i.
        //   To get the tightest UB on x_j (when a_j > 0), we need the MINIMUM of b - sum,
        //   which means maximizing sum. So use ub for positive a_i, lb for negative a_i.
        //   Wait... to MINIMIZE (b - sum), we MAXIMIZE sum.
        //   sum_max: for a_i > 0 use ub_i, for a_i < 0 use lb_i.
        //
        // For Ge: a_j*x_j + ... >= b → a_j*x_j >= b - sum_{i!=j} a_i*x_i.
        //   To get tightest LB on x_j (when a_j > 0), we need MAXIMUM of b - sum,
        //   which means MINIMIZING sum. So use lb for positive a_i, ub for negative a_i.

        let mut sum = Rat::zero();
        let mut bound_mults: Vec<(ConIndex, Rat)> = Vec::new();
        let mut all_finite = true;

        for &var_i in var_indices {
            if var_i == var_j {
                continue;
            }
            let a_i = &con.terms[&var_i];
            if a_i.is_zero() {
                continue;
            }

            // Which bound do we need?
            let need_ub_for_sum = match dir {
                Sense::Le => a_i.is_positive(),  // Le: maximize sum → ub for positive coeff
                Sense::Ge => a_i.is_negative(),  // Ge: minimize sum → ub for negative coeff
                Sense::Eq => unreachable!(),
            };

            let (lb_ci, ub_ci) = bound_map.get(&var_i).copied().unwrap_or((None, None));

            let (bound_ci, bound_val) = if need_ub_for_sum {
                // Need UB for variable i
                match ub_ci {
                    Some(ci) => (ci, &cert.get_constraint(ci).rhs),
                    None => { all_finite = false; break; }
                }
            } else {
                // Need LB for variable i
                match lb_ci {
                    Some(ci) => (ci, &cert.get_constraint(ci).rhs),
                    None => { all_finite = false; break; }
                }
            };

            sum += a_i * bound_val;

            // Multiplier on this bound constraint for the {lin} derivation.
            // We'll compute the full multiplier set later.
            bound_mults.push((bound_ci, a_i.clone()));
        }

        if !all_finite {
            continue;
        }

        // Implied value: (b - sum) / a_j
        let remainder = &con.rhs - &sum;
        let implied_val = &remainder / a_j;

        // Determine what kind of bound this gives us.
        // Le + a_j > 0 → UB, Le + a_j < 0 → LB (division reverses)
        // Ge + a_j > 0 → LB, Ge + a_j < 0 → UB (division reverses)
        let is_upper_bound = match dir {
            Sense::Le => a_j.is_positive(),
            Sense::Ge => a_j.is_negative(),
            Sense::Eq => unreachable!(),
        };

        // For integer variables, we can tighten further.
        let var = &instance.variables[var_j.0];
        let implied_int = if var.is_integer {
            if is_upper_bound {
                crate::rational::floor(&implied_val)
            } else {
                crate::rational::ceil(&implied_val)
            }
        } else {
            implied_val.clone()
        };

        // Check if this is tighter than the current bound.
        let (lb_ci, ub_ci) = bound_map.get(&var_j).copied().unwrap_or((None, None));

        let is_tighter = if is_upper_bound {
            match ub_ci {
                Some(ci) => implied_int < cert.get_constraint(ci).rhs,
                None => true, // no current UB — any finite UB is tighter
            }
        } else {
            match lb_ci {
                Some(ci) => implied_int > cert.get_constraint(ci).rhs,
                None => true, // no current LB — any finite LB is tighter
            }
        };

        if !is_tighter {
            continue;
        }

        // Build the {lin} derivation.
        // The linear combination: (1/a_j) * constraint + sum of bound adjustments.
        // We want to derive: x_j [sense] implied_val
        //
        // From: a_j*x_j + sum a_i*x_i [dir] b
        // Multiply by 1/a_j (flip sense if a_j < 0):
        //   x_j + sum (a_i/a_j)*x_i [derived_sense] b/a_j
        //
        // Then subtract each (a_i/a_j)*bound_i to zero out x_i:
        //   x_j [derived_sense] b/a_j - sum (a_i/a_j)*bound_val_i

        let one_over_aj = Rat::new(a_j.denom().clone(), a_j.numer().clone());
        let derived_sense = if is_upper_bound { Sense::Le } else { Sense::Ge };

        // Build multiplier list for {lin}.
        let mut multipliers: Vec<(ConIndex, Rat)> = Vec::new();

        // Multiplier on the constraint itself: 1/a_j.
        // For suitability: lambda * s(con) must have correct sign.
        // If derived_sense = Le: all lambda*s must be <= 0.
        // If derived_sense = Ge: all lambda*s must be >= 0.
        multipliers.push((con_ci, one_over_aj.clone()));

        // Multipliers on bound constraints to zero out other variables.
        for (bound_ci, a_i) in &bound_mults {
            // Multiplier to subtract (a_i/a_j) * bound_constraint from the combination.
            // The bound constraint is: x_i [>= lb | <= ub].
            // We need to add a multiplier that cancels (a_i/a_j) * x_i.
            let mult = -(a_i / a_j);
            if !mult.is_zero() {
                multipliers.push((*bound_ci, mult));
            }
        }

        // Verify: compute actual LHS and RHS from multipliers.
        let mut lhs: BTreeMap<VarIndex, Rat> = BTreeMap::new();
        let mut rhs = Rat::zero();
        for &(ci, ref lambda) in &multipliers {
            let c = cert.get_constraint(ci);
            for (&v, coeff) in &c.terms {
                *lhs.entry(v).or_insert_with(Rat::zero) += lambda * coeff;
            }
            rhs += lambda * &c.rhs;
        }
        lhs.retain(|_, v| !v.is_zero());

        // LHS should be just { var_j: 1 }.
        let expected_lhs: BTreeMap<VarIndex, Rat> = {
            let mut m = BTreeMap::new();
            m.insert(var_j, rat_from_int(1));
            m
        };
        if lhs != expected_lhs {
            // Rounding errors in the multipliers — skip this bound.
            continue;
        }

        // Verify suitability.
        let suit_ok = multipliers.iter().all(|(ci, lambda)| {
            let c = cert.get_constraint(*ci);
            let sign = suitability_sign(lambda, c.sense);
            match derived_sense {
                Sense::Ge => sign >= 0,
                Sense::Le => sign <= 0,
                Sense::Eq => unreachable!(),
            }
        });

        if !suit_ok {
            continue;
        }

        // Create the {lin} derivation.
        let lin_con = Constraint {
            name: format!("d{}", cert.next_index().0 - cert.con_count()),
            sense: derived_sense,
            rhs: rhs.clone(),
            terms: expected_lhs.clone(),
        };

        stats.num_lin_steps += 1;
        let lin_idx = cert.add_derivation(
            lin_con.clone(),
            Reason::Lin(multipliers),
            std::collections::BTreeSet::new(), // empty assumptions — globally valid
        );

        // Apply {rnd} if integer variable and rhs is non-integer.
        let (final_idx, final_con) = if var.is_integer {
            if let Some((mut rnd_con, rnd_reason)) = apply_rnd(&lin_con, lin_idx) {
                rnd_con.name = format!("d{}", cert.next_index().0 - cert.con_count());
                stats.num_rnd_steps += 1;
                let rnd_idx = cert.add_derivation(
                    rnd_con.clone(),
                    rnd_reason,
                    std::collections::BTreeSet::new(),
                );
                (rnd_idx, rnd_con)
            } else {
                (lin_idx, lin_con)
            }
        } else {
            (lin_idx, lin_con)
        };

        // Add permanent bound to oracle.
        oracle.add_permanent_bound(var_j, derived_sense, &final_con.rhs, final_idx)?;

        // Update bound_map.
        let entry = bound_map.entry(var_j).or_insert((None, None));
        if is_upper_bound {
            entry.1 = Some(final_idx);
        } else {
            entry.0 = Some(final_idx);
        }

        stats.num_propagated_bounds += 1;
        count += 1;

        println!(
            "    Prop: {} {} {} (from {})",
            instance.variables[var_j.0].name,
            if is_upper_bound { "<=" } else { ">=" },
            crate::vipr::fmt_rational(&final_con.rhs),
            con.name,
        );
    }

    if count > 0 { Ok(Some(count)) } else { Ok(None) }
}

/// Compute the sign of λ · s(C).
fn suitability_sign(lambda: &Rat, sense: Sense) -> i8 {
    match sense {
        Sense::Eq => 0,
        Sense::Ge => {
            if lambda.is_positive() { 1 } else if lambda.is_negative() { -1 } else { 0 }
        }
        Sense::Le => {
            if lambda.is_positive() { -1 } else if lambda.is_negative() { 1 } else { 0 }
        }
    }
}
