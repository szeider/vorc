//! Exact dual computation and verification.
//!
//! Both bound derivation (OBJ ≥ z) and Farkas derivation (0 ≥ β > 0) share
//! a common core [`derive_lin_core`]: rationalize multipliers, repair LHS to
//! match a target, and verify suitability. The cascade wrapper
//! [`cascade_rationalize`] drives increasing denominator bounds (10^6..10^18).

use crate::rational::{ceil, floor, rat_from_float, rat_from_int, Rat};
use crate::types::{ConIndex, Constraint, ObjSense, Reason, Sense, VarIndex, Variable};
use anyhow::{anyhow, bail, Result};
use num_traits::{Signed, Zero};
use std::collections::BTreeMap;

/// Look up a constraint by index: CON section by position, branch constraints from `extra`.
fn lookup_constraint<'a>(
    idx: ConIndex,
    constraints: &'a [Constraint],
    extra: &'a BTreeMap<ConIndex, Constraint>,
) -> Result<&'a Constraint> {
    if idx.0 < constraints.len() {
        Ok(&constraints[idx.0])
    } else {
        extra
            .get(&idx)
            .ok_or_else(|| anyhow!("unknown ConIndex({})", idx.0))
    }
}

/// Compute the sign of λ · s(C) without allocating a BigRational.
/// Returns +1 if the suitability product is positive, -1 if negative, 0 if zero.
fn suitability_sign(lambda: &Rat, sense: Sense) -> i8 {
    match sense {
        Sense::Eq => 0, // s(Eq) = 0, so product is always 0
        Sense::Ge => {
            // s(Ge) = +1, so sign(λ · s) = sign(λ)
            if lambda.is_positive() { 1 } else if lambda.is_negative() { -1 } else { 0 }
        }
        Sense::Le => {
            // s(Le) = -1, so sign(λ · s) = -sign(λ)
            if lambda.is_positive() { -1 } else if lambda.is_negative() { 1 } else { 0 }
        }
    }
}

/// Denominator bounds for cascaded rationalization: 10^6, 10^7, ..., 10^18.
const CASCADE_DENOMS: &[i64] = &[
    1_000_000,                   // 10^6
    10_000_000,                  // 10^7
    100_000_000,                 // 10^8
    1_000_000_000,               // 10^9
    10_000_000_000,              // 10^10
    100_000_000_000,             // 10^11
    1_000_000_000_000,           // 10^12
    10_000_000_000_000,          // 10^13
    100_000_000_000_000,         // 10^14
    1_000_000_000_000_000,       // 10^15
    10_000_000_000_000_000,      // 10^16
    100_000_000_000_000_000,     // 10^17
    1_000_000_000_000_000_000,   // 10^18
];

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// Result of derive_bound: the derived constraint, its reason, and whether {rnd} is applicable.
pub struct DeriveResult {
    pub constraint: Constraint,
    pub reason: Reason,
    pub rnd_applicable: bool,
    /// The denominator bound level that succeeded (index into CASCADE_DENOMS).
    pub cascade_level: usize,
}

/// Result of derive_farkas: an absurdity constraint (empty LHS, contradictory RHS).
pub struct FarkasResult {
    pub constraint: Constraint,
    pub reason: Reason,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Derive an objective bound from LP duals using cascaded rationalization.
///
/// Tries continued-fraction approximation with denominator bounds 10^6, 10^7, ..., 10^18.
/// Returns the first level that produces a verified {lin} step, or error if all fail.
/// Derive an objective bound from LP dual multipliers.
///
/// When `pruning_target` is `Some(z*)`, the cascade rejects any derivation whose
/// bound (after optional `{rnd}`) does not meet z*. This forces higher cascade
/// levels to produce a tighter rationalization, which matters when the LP solution
/// is integral and cannot be branched on.
pub fn derive_bound(
    duals: &BTreeMap<ConIndex, f64>,
    obj_sense: ObjSense,
    obj_terms: &BTreeMap<VarIndex, Rat>,
    constraints: &[Constraint],
    extra: &BTreeMap<ConIndex, Constraint>,
    variables: &[Variable],
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    pruning_target: Option<&Rat>,
) -> Result<DeriveResult> {
    let target_sense = match obj_sense {
        ObjSense::Minimize => Sense::Ge,
        ObjSense::Maximize => Sense::Le,
    };

    let (result, level) = cascade_rationalize(duals, "Cascade", |rat_duals| {
        let (constraint, reason) = derive_lin_core(
            rat_duals, obj_terms, target_sense, constraints, extra, bound_map,
        )?;

        let rnd_applicable = constraint.terms.iter().all(|(var_idx, coeff)| {
            if variables[var_idx.0].is_integer {
                coeff.is_integer()
            } else {
                coeff.is_zero()
            }
        });

        // If a pruning target is set, check the bound (after optional rnd).
        if let Some(z_star) = pruning_target {
            let effective_rhs = if rnd_applicable {
                use crate::rational::ceil;
                match target_sense {
                    Sense::Ge => ceil(&constraint.rhs),
                    Sense::Le => crate::rational::floor(&constraint.rhs),
                    Sense::Eq => constraint.rhs.clone(),
                }
            } else {
                constraint.rhs.clone()
            };
            let meets = match obj_sense {
                ObjSense::Minimize => effective_rhs >= *z_star,
                ObjSense::Maximize => effective_rhs <= *z_star,
            };
            if !meets {
                anyhow::bail!("bound {} does not meet pruning target {}",
                    crate::vipr::fmt_rational(&effective_rhs),
                    crate::vipr::fmt_rational(z_star));
            }
        }

        Ok((constraint, reason, rnd_applicable))
    })?;
    let (constraint, reason, rnd_applicable) = result;
    Ok(DeriveResult { constraint, reason, rnd_applicable, cascade_level: level })
}

/// Derive absurdity from Farkas certificate of an infeasible LP node.
///
/// Uses cascade rationalization (10^6 through 10^18) to find working multipliers.
/// Tries both original and negated multipliers at each level, since LHS repair
/// can break suitability for one sense but work for the other.
/// Returns a {lin} derivation with empty LHS and contradictory RHS.
pub fn derive_farkas(
    farkas: &BTreeMap<ConIndex, f64>,
    constraints: &[Constraint],
    extra: &BTreeMap<ConIndex, Constraint>,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
) -> Result<FarkasResult> {
    let empty_target: BTreeMap<VarIndex, Rat> = BTreeMap::new();

    let (result, _) = cascade_rationalize(farkas, "Farkas", |rat_farkas| {
        // Try original multipliers, then negated.  LHS repair can break
        // suitability for one sense even when determine_combo_sense succeeds,
        // so we attempt both directions at every cascade level.
        let negated: BTreeMap<ConIndex, Rat> = rat_farkas
            .iter()
            .map(|(&k, v)| (k, -v))
            .collect();

        let attempts: [(_, &str); 2] = [
            (rat_farkas, "original"),
            (negated, "negated"),
        ];

        let mut last_err = None;
        for (mults, label) in attempts {
            let combo_sense = match determine_combo_sense(&mults, constraints, extra) {
                Ok(Some(s)) => s,
                Ok(None) => continue, // mixed suitability, skip
                Err(e) => { last_err = Some(e); continue; }
            };

            match try_farkas_lin(
                mults, &empty_target, combo_sense, constraints, extra, bound_map,
            ) {
                Ok(result) => {
                    if label == "negated" {
                        println!("    → Farkas succeeded with negated multipliers");
                    }
                    return Ok(result);
                }
                Err(e) => { last_err = Some(e); }
            }
        }

        bail!(
            "Farkas: both original and negated multipliers failed. Last: {}",
            last_err.map_or("no attempts".into(), |e| format!("{e:#}"))
        )
    })?;
    Ok(result)
}

/// Try a single Farkas {lin} derivation with given multipliers and sense.
fn try_farkas_lin(
    rat_farkas: BTreeMap<ConIndex, Rat>,
    empty_target: &BTreeMap<VarIndex, Rat>,
    combo_sense: Sense,
    constraints: &[Constraint],
    extra: &BTreeMap<ConIndex, Constraint>,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
) -> Result<FarkasResult> {
    let (constraint, reason) = derive_lin_core(
        rat_farkas, empty_target, combo_sense, constraints, extra, bound_map,
    )?;

    // Verify absurdity: RHS must contradict empty LHS.
    match combo_sense {
        Sense::Ge => {
            if !constraint.rhs.is_positive() {
                bail!("Farkas absurdity failed: 0 >= {} (need positive RHS)", constraint.rhs);
            }
        }
        Sense::Le => {
            if !constraint.rhs.is_negative() {
                bail!("Farkas absurdity failed: 0 <= {} (need negative RHS)", constraint.rhs);
            }
        }
        Sense::Eq => unreachable!(),
    }

    Ok(FarkasResult { constraint, reason })
}

/// Apply integer rounding ({rnd}) to a {lin}-derived constraint.
///
/// For Ge: ceil(rhs). For Le: floor(rhs).
/// Returns None if rounding doesn't tighten the bound.
pub fn apply_rnd(
    lin_constraint: &Constraint,
    lin_idx: ConIndex,
) -> Option<(Constraint, Reason)> {
    let new_rhs = match lin_constraint.sense {
        Sense::Ge => ceil(&lin_constraint.rhs),
        Sense::Le => floor(&lin_constraint.rhs),
        Sense::Eq => return None, // no rounding on equality
    };

    if new_rhs == lin_constraint.rhs {
        return None; // no tightening
    }

    let constraint = Constraint {
        name: String::new(), // caller assigns name
        sense: lin_constraint.sense,
        rhs: new_rhs,
        terms: lin_constraint.terms.clone(),
    };

    let reason = Reason::Rnd(vec![(lin_idx, rat_from_int(1))]);

    Some((constraint, reason))
}

// ---------------------------------------------------------------------------
// Core derivation (shared by bound and Farkas)
// ---------------------------------------------------------------------------

/// Core {lin} derivation: given rational multipliers and a target LHS,
/// compute the linear combination, repair LHS via bound constraints,
/// and verify suitability.
///
/// For bound derivation, `target_lhs` is the objective coefficients.
/// For Farkas, `target_lhs` is empty (deriving an absurdity 0 ≥ β).
fn derive_lin_core(
    mut rat_mults: BTreeMap<ConIndex, Rat>,
    target_lhs: &BTreeMap<VarIndex, Rat>,
    target_sense: Sense,
    constraints: &[Constraint],
    extra: &BTreeMap<ConIndex, Constraint>,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
) -> Result<(Constraint, Reason)> {
    // Step 1: Compute LHS and RHS in a single pass.
    let mut lhs: BTreeMap<VarIndex, Rat> = BTreeMap::new();
    let mut rhs = Rat::zero();
    for (&idx, lambda) in &rat_mults {
        let con = lookup_constraint(idx, constraints, extra)?;
        for (&var, coeff) in &con.terms {
            let entry = lhs.entry(var).or_insert_with(Rat::zero);
            *entry = &*entry + &(lambda * coeff);
        }
        rhs += lambda * &con.rhs;
    }
    lhs.retain(|_, v| !v.is_zero());

    // Step 2: Constraint elimination for variables without suitable repair bounds.
    // When a variable has no bound constraint that can absorb the deficit (e.g., free
    // variables with -∞..+∞ domain), adjust the multiplier on a contributing constraint
    // to exactly zero out that variable's LHS coefficient.  This changes other variables'
    // contributions (which may expose new unrepairable variables), so we iterate.
    const MAX_ELIM_ROUNDS: usize = 50;
    for _round in 0..MAX_ELIM_ROUNDS {
        let elim_vars: Vec<(VarIndex, Rat)> = lhs
            .iter()
            .filter_map(|(&var, lhs_coeff)| {
                let target_coeff = target_lhs.get(&var).cloned().unwrap_or_else(Rat::zero);
                let deficit = &target_coeff - lhs_coeff;
                if deficit.is_zero() {
                    return None;
                }
                let (lb_idx, ub_idx) = bound_map.get(&var).copied().unwrap_or((None, None));
                let has_suitable_bound = match target_sense {
                    Sense::Ge => {
                        if deficit.is_positive() { lb_idx.is_some() } else { ub_idx.is_some() }
                    }
                    Sense::Le => {
                        if deficit.is_negative() { lb_idx.is_some() } else { ub_idx.is_some() }
                    }
                    Sense::Eq => false,
                };
                if has_suitable_bound { None } else { Some((var, deficit)) }
            })
            .collect();

        if elim_vars.is_empty() {
            break;
        }

        for (var, deficit) in elim_vars {
            // Find the constraint in the combination with the largest |λ·coeff(var)|
            // to minimize relative perturbation of the multiplier.
            let mut best: Option<(ConIndex, Rat, Rat)> = None;
            for (&idx, lambda) in &rat_mults {
                let con = lookup_constraint(idx, constraints, extra)?;
                if let Some(coeff) = con.terms.get(&var) {
                    if !coeff.is_zero() {
                        let weight = (lambda * coeff).abs();
                        let dominated = best.as_ref().map_or(false, |(_, _, w)| weight <= *w);
                        if !dominated {
                            best = Some((idx, coeff.clone(), weight));
                        }
                    }
                }
            }
            let (elim_idx, coeff) = match best {
                Some((idx, c, _)) => (idx, c),
                None => bail!(
                    "Constraint elimination failed for var {:?}: no constraint in combination contains it",
                    var
                ),
            };
            let adjustment = &deficit / &coeff;
            let con = lookup_constraint(elim_idx, constraints, extra)?;
            for (&v, c) in &con.terms {
                *lhs.entry(v).or_insert_with(Rat::zero) += &adjustment * c;
            }
            rhs += &adjustment * &con.rhs;
            *rat_mults.entry(elim_idx).or_insert_with(Rat::zero) += &adjustment;
        }
        lhs.retain(|_, v| !v.is_zero());
        rat_mults.retain(|_, v| !v.is_zero());
    }

    // Step 3: LHS repair — adjust coefficients to match target via bound constraints.
    let mut repairs: Vec<(VarIndex, Rat)> = Vec::new();

    // Variables in target but wrong/missing in lhs.
    for (&var, target_coeff) in target_lhs {
        let lhs_coeff = lhs.get(&var).cloned().unwrap_or_else(Rat::zero);
        let deficit = target_coeff - &lhs_coeff;
        if !deficit.is_zero() {
            repairs.push((var, deficit));
        }
    }

    // Variables in lhs but not in target (should be zero).
    for (&var, lhs_coeff) in &lhs {
        if !target_lhs.contains_key(&var) && !lhs_coeff.is_zero() {
            repairs.push((var, -lhs_coeff));
        }
    }

    for (var, deficit) in repairs {
        let (lb_idx, ub_idx) = bound_map.get(&var).copied().unwrap_or((None, None));

        // Choose the bound that preserves suitability.  Never fall back to
        // the opposite bound — that would introduce a wrong-sign multiplier.
        let repair_idx = match target_sense {
            Sense::Ge => {
                if deficit.is_positive() { lb_idx } else { ub_idx }
            }
            Sense::Le => {
                if deficit.is_negative() { lb_idx } else { ub_idx }
            }
            Sense::Eq => unreachable!(),
        };

        let con_idx = match repair_idx {
            Some(idx) => idx,
            None => bail!(
                "Cannot repair LHS for var {:?}: deficit={}, no suitable bound",
                var, deficit
            ),
        };

        *rat_mults.entry(con_idx).or_insert_with(Rat::zero) += &deficit;
        *lhs.entry(var).or_insert_with(Rat::zero) += &deficit;
        rhs += &deficit * &lookup_constraint(con_idx, constraints, extra)?.rhs;
    }

    // Clean up zeros.
    rat_mults.retain(|_, v| !v.is_zero());
    lhs.retain(|_, v| !v.is_zero());

    // Step 4: Verify LHS matches target exactly.
    if lhs != *target_lhs {
        let mut details = String::new();
        for (var, coeff) in &lhs {
            let target = target_lhs.get(var);
            details.push_str(&format!(
                "\n  var {:?}: LHS={}, target={}",
                var,
                coeff,
                target.map_or("0".to_string(), |c| c.to_string())
            ));
        }
        for (var, coeff) in target_lhs {
            if !lhs.contains_key(var) {
                details.push_str(&format!("\n  var {:?}: LHS=0, target={}", var, coeff));
            }
        }
        bail!("LHS does not match target after repair{details}");
    }

    // Step 5: Verify suitability — all λ_i · s(C_i) must have consistent sign.
    for (&idx, lambda) in &rat_mults {
        let con = lookup_constraint(idx, constraints, extra)?;
        let sign = suitability_sign(lambda, con.sense);
        match target_sense {
            Sense::Ge => {
                if sign < 0 {
                    bail!("Suitability failed: {} (need λ·s ≥ 0)", con.name);
                }
            }
            Sense::Le => {
                if sign > 0 {
                    bail!("Suitability failed: {} (need λ·s ≤ 0)", con.name);
                }
            }
            Sense::Eq => unreachable!(),
        }
    }

    // Build result.
    let multipliers: Vec<(ConIndex, Rat)> = rat_mults.into_iter().collect();

    let constraint = Constraint {
        name: String::new(),
        sense: target_sense,
        rhs,
        terms: lhs,
    };

    Ok((constraint, Reason::Lin(multipliers)))
}

// ---------------------------------------------------------------------------
// Cascade helper
// ---------------------------------------------------------------------------

/// Try rationalization at increasing denominator bounds until one succeeds.
fn cascade_rationalize<T>(
    float_mults: &BTreeMap<ConIndex, f64>,
    label: &str,
    attempt: impl Fn(BTreeMap<ConIndex, Rat>) -> Result<T>,
) -> Result<(T, usize)> {
    let mut last_err = None;
    for (level, &max_denom) in CASCADE_DENOMS.iter().enumerate() {
        let mut rat_mults: BTreeMap<ConIndex, Rat> = BTreeMap::new();
        for (&idx, &val) in float_mults {
            let r = rat_from_float(val, max_denom);
            if !r.is_zero() {
                rat_mults.insert(idx, r);
            }
        }
        if rat_mults.is_empty() {
            continue;
        }
        match attempt(rat_mults) {
            Ok(result) => {
                if level > 0 {
                    println!("    → {label} succeeded at max_denom={max_denom}");
                }
                return Ok((result, level));
            }
            Err(e) => last_err = Some(e),
        }
    }
    bail!(
        "{label}: all rationalization levels (10^6..10^18) failed. Last error: {}",
        last_err.map_or("all duals zero".to_string(), |e| format!("{e:#}"))
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine combination sense from λ·s(C) products.
/// Returns Some(Ge) if all non-negative with some positive,
/// Some(Le) if all non-positive with some negative,
/// None if mixed.
fn determine_combo_sense(
    multipliers: &BTreeMap<ConIndex, Rat>,
    constraints: &[Constraint],
    extra: &BTreeMap<ConIndex, Constraint>,
) -> Result<Option<Sense>> {
    let mut all_nonneg = true;
    let mut all_nonpos = true;
    let mut any_pos = false;
    let mut any_neg = false;

    for (&idx, lambda) in multipliers {
        let con = lookup_constraint(idx, constraints, extra)?;
        let sign = suitability_sign(lambda, con.sense);
        if sign > 0 {
            all_nonpos = false;
            any_pos = true;
        } else if sign < 0 {
            all_nonneg = false;
            any_neg = true;
        }
    }

    if all_nonneg && any_pos {
        Ok(Some(Sense::Ge))
    } else if all_nonpos && any_neg {
        Ok(Some(Sense::Le))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_lp;
    use crate::types::{ConPlan, LpResult};
    use crate::oracle::Oracle;

    fn test_instances_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/instances")
    }

    #[test]
    fn test_derive_bound_tiny1() {
        let path = test_instances_dir().join("tiny1.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        let lp = oracle.solve_lp().unwrap();
        let (duals, _obj_val) = match lp {
            LpResult::Optimal { duals, obj_val, .. } => (duals, obj_val),
            _ => panic!("Expected Optimal"),
        };

        let result = derive_bound(
            &duals,
            inst.obj_sense,
            &inst.obj_terms,
            &plan.constraints,
            &BTreeMap::new(),
            &inst.variables,
            &plan.bound_map,
            None,
        )
        .unwrap();

        // Should derive OBJ >= 4/3.
        assert_eq!(result.constraint.sense, Sense::Ge);
        use crate::rational::rat;
        assert_eq!(result.constraint.rhs, rat(4, 3));
        assert!(result.rnd_applicable, "should be rnd-applicable (all integer vars)");
    }

    #[test]
    fn test_apply_rnd_tiny1() {
        use crate::rational::rat;
        let lin_con = Constraint {
            name: "d0".into(),
            sense: Sense::Ge,
            rhs: rat(4, 3),
            terms: {
                let mut t = BTreeMap::new();
                t.insert(VarIndex(0), rat_from_int(1));
                t.insert(VarIndex(1), rat_from_int(1));
                t
            },
        };
        let result = apply_rnd(&lin_con, ConIndex(6));
        assert!(result.is_some(), "should tighten 4/3 to 2");
        let (rnd_con, _reason) = result.unwrap();
        assert_eq!(rnd_con.rhs, rat_from_int(2));
        assert_eq!(rnd_con.sense, Sense::Ge);
    }

    #[test]
    fn test_derive_bound_cg() {
        let path = test_instances_dir().join("cg.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        let lp = oracle.solve_lp().unwrap();
        let duals = match lp {
            LpResult::Optimal { duals, .. } => duals,
            _ => panic!("Expected Optimal"),
        };

        let result = derive_bound(
            &duals,
            inst.obj_sense,
            &inst.obj_terms,
            &plan.constraints,
            &BTreeMap::new(),
            &inst.variables,
            &plan.bound_map,
            None,
        )
        .unwrap();

        // Should derive OBJ >= 1/4.
        assert_eq!(result.constraint.sense, Sense::Ge);
        use crate::rational::rat;
        assert_eq!(result.constraint.rhs, rat(1, 4));
        assert!(result.rnd_applicable);

        // rnd should tighten to 1.
        let rnd = apply_rnd(&result.constraint, ConIndex(6));
        assert!(rnd.is_some());
        let (rnd_con, _) = rnd.unwrap();
        assert_eq!(rnd_con.rhs, rat_from_int(1));
    }

    #[test]
    fn test_derive_farkas_tiny2() {
        // tiny2: max 3x+4y, 2x+3y<=4, binary x,y.
        // Push x>=1 and y>=1 → infeasible (2+3=5>4).
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        // Branch constraints as DER entries.
        let branch_x_idx = ConIndex(plan.constraints.len());     // first DER
        let branch_y_idx = ConIndex(plan.constraints.len() + 1); // second DER

        let rhs_one = rat_from_int(1);
        oracle
            .push_branch_constraint(VarIndex(0), Sense::Ge, &rhs_one, branch_x_idx)
            .unwrap();
        oracle
            .push_branch_constraint(VarIndex(1), Sense::Ge, &rhs_one, branch_y_idx)
            .unwrap();

        let result = oracle.solve_lp().unwrap();
        let farkas = match result {
            LpResult::Infeasible { farkas } => farkas,
            _ => panic!("Expected Infeasible"),
        };

        // Build extra map for branch constraints.
        let mut extra = BTreeMap::new();
        let mut terms_x = BTreeMap::new();
        terms_x.insert(VarIndex(0), rat_from_int(1));
        extra.insert(
            branch_x_idx,
            Constraint {
                name: "branch_x".into(),
                sense: Sense::Ge,
                rhs: rhs_one.clone(),
                terms: terms_x,
            },
        );
        let mut terms_y = BTreeMap::new();
        terms_y.insert(VarIndex(1), rat_from_int(1));
        extra.insert(
            branch_y_idx,
            Constraint {
                name: "branch_y".into(),
                sense: Sense::Ge,
                rhs: rhs_one.clone(),
                terms: terms_y,
            },
        );

        let farkas_result = derive_farkas(
            &farkas,
            &plan.constraints,
            &extra,
            &plan.bound_map,
        )
        .unwrap();

        // Should be an absurdity: empty LHS with contradictory RHS.
        assert!(
            farkas_result.constraint.terms.is_empty(),
            "Farkas LHS should be empty (absurdity)"
        );

        // Verify RHS is contradictory: Ge with positive RHS, or Le with negative RHS.
        match farkas_result.constraint.sense {
            Sense::Ge => assert!(
                farkas_result.constraint.rhs.is_positive(),
                "Ge absurdity needs positive RHS, got {}",
                farkas_result.constraint.rhs
            ),
            Sense::Le => assert!(
                farkas_result.constraint.rhs.is_negative(),
                "Le absurdity needs negative RHS, got {}",
                farkas_result.constraint.rhs
            ),
            _ => panic!("Absurdity should be Ge or Le"),
        }

        // Reason should be Lin.
        match &farkas_result.reason {
            Reason::Lin(mults) => {
                assert!(!mults.is_empty(), "should have multipliers");
            }
            _ => panic!("Expected Lin reason"),
        }

        oracle.pop_branch_constraint().unwrap();
        oracle.pop_branch_constraint().unwrap();
    }
}
