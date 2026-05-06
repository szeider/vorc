//! Branch-and-bound tree traversal.
//!
//! Drives the external B&B: at each node, queries the oracle for LP solution,
//! decides pruning/branching, and builds the VIPR derivation tree.

use crate::dual::{apply_rnd, derive_bound, derive_farkas};
use crate::oracle::Oracle;
use crate::rational::{ceil, floor, rat_from_int, Rat};
use crate::types::{
    ConIndex, Constraint, Instance, LpResult, NodeResult, ObjSense, Reason, RunStats, Sense,
    VarIndex,
};
use crate::vipr::CertificateBuilder;
use anyhow::{bail, Result};
use num_traits::Zero;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

/// Tolerance for checking whether a variable value is integral.
const INTEGRALITY_TOL: f64 = 1e-6;

/// Tight tolerance for near-integral LP fallback.
/// When the LP appears integral at normal tolerance but the bound doesn't meet z*
/// (e.g., maximization with bound = z*+1), we try this tighter tolerance to find
/// a nearly-fractional variable to branch on.
const TIGHT_INTEGRALITY_TOL: f64 = 1e-12;

/// Reliability branching: strong-branch until this many observations per direction.
const RELIABILITY_THRESHOLD: u32 = 5;

/// Maximum candidates to strong-branch per node in reliability branching.
const MAX_STRONG_CANDIDATES: usize = 5;

/// Weight for the max component in reliability scoring (mu).
const RELIABILITY_MU: f64 = 0.2;

/// Check if objective has integer-valued structure:
/// all non-zero coefficients are integers on integer variables.
fn obj_integer_valued(instance: &Instance) -> bool {
    instance.obj_terms.iter().all(|(vi, coeff)| {
        coeff.is_zero() || (instance.variables[vi.0].is_integer && coeff.is_integer())
    })
}

/// Per-variable pseudocost tracking.
#[derive(Clone, Default)]
struct PseudocostEntry {
    down_sum: f64,
    down_cnt: u32,
    up_sum: f64,
    up_cnt: u32,
}

/// Branching state threaded through B&B.
struct BranchingState {
    pseudocosts: BTreeMap<VarIndex, PseudocostEntry>,
}

/// Effective bounds for each variable: (effective_lb, effective_ub).
/// Each bound is (value, constraint_index) where ConIndex points to the
/// CON bound row or DER {asm} from branching/propagation.
type EffBounds = BTreeMap<VarIndex, (Option<(Rat, ConIndex)>, Option<(Rat, ConIndex)>)>;

/// Initialize effective bounds from root bounds (original + propagated).
fn init_eff_bounds(
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    cert: &CertificateBuilder,
) -> EffBounds {
    let mut eff = BTreeMap::new();
    for (i, _var) in instance.variables.iter().enumerate() {
        let vi = VarIndex(i);
        let (lb_ci, ub_ci) = bound_map.get(&vi).copied().unwrap_or((None, None));
        let eff_lb = lb_ci.map(|ci| (cert.get_constraint(ci).rhs.clone(), ci));
        let eff_ub = ub_ci.map(|ci| (cert.get_constraint(ci).rhs.clone(), ci));
        eff.insert(vi, (eff_lb, eff_ub));
    }
    eff
}

/// Apply a branch constraint to effective bounds (tighten if appropriate).
fn apply_branch_to_eff_bounds(
    eff: &mut EffBounds,
    var: VarIndex,
    sense: Sense,
    rhs: &Rat,
    asm_idx: ConIndex,
) {
    let entry = eff.entry(var).or_insert((None, None));
    match sense {
        Sense::Le => {
            if entry.1.as_ref().map_or(true, |(val, _)| *rhs < *val) {
                entry.1 = Some((rhs.clone(), asm_idx));
            }
        }
        Sense::Ge => {
            if entry.0.as_ref().map_or(true, |(val, _)| *rhs > *val) {
                entry.0 = Some((rhs.clone(), asm_idx));
            }
        }
        Sense::Eq => {}
    }
}

/// Compute effective bounds from root bounds + a sequence of branch constraints.
fn compute_eff_bounds_with_branches(
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    cert: &CertificateBuilder,
    branches: &[BranchInfo],
) -> EffBounds {
    let mut eff = init_eff_bounds(instance, bound_map, cert);
    for b in branches {
        apply_branch_to_eff_bounds(&mut eff, b.var, b.sense, &b.rhs, b.asm_idx);
    }
    eff
}

/// Build a bound_map from effective bounds for use in derive_bound/derive_farkas.
/// This gives LHS repair access to tighter node-specific bounds (from branching/propagation).
fn eff_bound_map(eff: &EffBounds) -> BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)> {
    eff.iter()
        .map(|(&var, (lb, ub))| {
            (var, (lb.as_ref().map(|(_, ci)| *ci), ub.as_ref().map(|(_, ci)| *ci)))
        })
        .collect()
}

/// Attempt CP-style pruning at a B&B node before calling the LP oracle.
/// Check 1: domain infeasibility (lb > ub for any variable).
/// Check 2: objective bound from variable bounds meets z* (skipped if z_star is None).
/// Returns Some(NodeResult) if pruned, None if LP solve is needed.
fn try_cp_pruning(
    cert: &mut CertificateBuilder,
    instance: &Instance,
    eff_bounds: &EffBounds,
    z_star: Option<&Rat>,
    stats: &mut RunStats,
) -> Option<NodeResult> {
    // --- Check 1: Domain infeasibility ---
    for (_vi, (eff_lb, eff_ub)) in eff_bounds {
        if let (Some((lb_val, lb_ci)), Some((ub_val, ub_ci))) = (eff_lb, eff_ub) {
            if lb_val > ub_val {
                // 1*(x >= lb) + (-1)*(x <= ub) → 0 >= lb - ub > 0 → absurdity
                let reason = Reason::Lin(vec![
                    (*lb_ci, rat_from_int(1)),
                    (*ub_ci, rat_from_int(-1)),
                ]);
                let assumptions = collect_assumptions(cert, &reason);
                let con = Constraint {
                    name: next_der_name(cert),
                    sense: Sense::Ge,
                    rhs: lb_val - ub_val,
                    terms: BTreeMap::new(),
                };
                stats.num_lin_steps += 1;
                stats.num_cp_domain_pruned += 1;
                let idx = cert.add_derivation(con, reason, assumptions);
                return Some(NodeResult::Absurdity(idx));
            }
        }
    }

    // --- Check 2: Objective bound from variable bounds (only when proving optimality) ---
    let z = z_star?;
    let zero = Rat::zero();
    let mut mults: Vec<(ConIndex, Rat)> = Vec::new();
    let mut bound_sum = Rat::zero();

    for (vi, coeff) in &instance.obj_terms {
        if coeff.is_zero() {
            continue;
        }
        let (eff_lb, eff_ub) = eff_bounds.get(vi)?;
        let needed = match instance.obj_sense {
            ObjSense::Minimize => {
                if *coeff > zero { eff_lb.as_ref() } else { eff_ub.as_ref() }
            }
            ObjSense::Maximize => {
                if *coeff > zero { eff_ub.as_ref() } else { eff_lb.as_ref() }
            }
        };
        let (bound_val, bound_ci) = needed?;
        mults.push((*bound_ci, coeff.clone()));
        bound_sum += coeff * bound_val;
    }

    // Pre-check: can this bound prune (possibly after rounding)?
    let rnd_applicable = instance.obj_terms.iter().all(|(vi, coeff)| {
        !instance.variables[vi.0].is_integer || coeff.is_integer()
    });

    let check_bound = if rnd_applicable {
        match instance.obj_sense {
            ObjSense::Minimize => ceil(&bound_sum),
            ObjSense::Maximize => floor(&bound_sum),
        }
    } else {
        bound_sum.clone()
    };

    let can_prune = match instance.obj_sense {
        ObjSense::Minimize => check_bound >= *z,
        ObjSense::Maximize => check_bound <= *z,
    };

    if !can_prune {
        return None;
    }

    // Build {lin} derivation.
    let target_sense = match instance.obj_sense {
        ObjSense::Minimize => Sense::Ge,
        ObjSense::Maximize => Sense::Le,
    };
    let reason = Reason::Lin(mults);
    let assumptions = collect_assumptions(cert, &reason);
    let lin_con = Constraint {
        name: next_der_name(cert),
        sense: target_sense,
        rhs: bound_sum,
        terms: instance.obj_terms.clone(),
    };
    stats.num_lin_steps += 1;
    let lin_idx = cert.add_derivation(lin_con.clone(), reason, assumptions);

    // Apply {rnd} if applicable.
    let final_idx = if rnd_applicable {
        if let Some((mut rnd_con, rnd_reason)) = apply_rnd(&lin_con, lin_idx) {
            rnd_con.name = next_der_name(cert);
            let rnd_assumptions = cert.get_assumptions(lin_idx).cloned().unwrap_or_default();
            stats.num_rnd_steps += 1;
            cert.add_derivation(rnd_con, rnd_reason, rnd_assumptions)
        } else {
            lin_idx
        }
    } else {
        lin_idx
    };

    stats.num_cp_bound_pruned += 1;
    Some(NodeResult::Bound(final_idx))
}

/// Run the full B&B traversal, producing VIPR derivations.
/// Returns the ConIndex of the final derivation (must have empty assumptions).
/// When z_star is None, proves infeasibility (all leaves must be Farkas absurdities).
pub fn run_bnb(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    max_nodes: usize,
    strong: bool,
    stats: &mut RunStats,
) -> Result<ConIndex> {
    let final_idx = run_bnb_core(
        oracle, cert, instance, bound_map, z_star, max_nodes, strong, stats,
        BTreeSet::new(),
    )?;

    // Verify empty assumptions.
    if let Some(asms) = cert.get_assumptions(final_idx)
        && !asms.is_empty()
    {
        bail!(
            "Final derivation has non-empty assumptions: {:?}",
            asms
        );
    }

    Ok(final_idx)
}

/// Core DFS B&B traversal without empty-assumption assertion.
/// `initial_asms` are pre-existing active assumptions (e.g., from objective split).
fn run_bnb_core(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    max_nodes: usize,
    strong: bool,
    stats: &mut RunStats,
    initial_asms: BTreeSet<ConIndex>,
) -> Result<ConIndex> {
    let mut eff_bounds = init_eff_bounds(instance, bound_map, cert);
    let mut branching = BranchingState { pseudocosts: BTreeMap::new() };
    let mut active_asms = initial_asms;
    let result = solve_node(
        oracle, cert, instance, bound_map, z_star, 0, stats, max_nodes, strong,
        &mut eff_bounds, &mut branching, &mut active_asms,
    )?;

    let final_idx = match result {
        NodeResult::Bound(idx) | NodeResult::Absurdity(idx) => idx,
    };

    println!("B&B complete: {} nodes explored", stats.num_bnb_nodes);
    Ok(final_idx)
}

/// Solve a single B&B node. Returns Bound or Absurdity.
fn solve_node(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    depth: usize,
    stats: &mut RunStats,
    max_nodes: usize,
    strong: bool,
    eff_bounds: &mut EffBounds,
    branching: &mut BranchingState,
    active_asms: &mut BTreeSet<ConIndex>,
) -> Result<NodeResult> {
    stats.num_bnb_nodes += 1;
    if max_nodes > 0 && stats.num_bnb_nodes > max_nodes {
        bail!(
            "Node limit reached ({max_nodes}). Use --max-nodes to increase. \
             Current depth: {depth}, derivations: {}",
            cert.next_index().0 - cert.con_count()
        );
    }
    let node_id = stats.num_bnb_nodes;

    // CP pruning check (before LP solve).
    if let Some(result) = try_cp_pruning(cert, instance, eff_bounds, z_star, stats) {
        println!("  Node {node_id} (depth {depth}): CP pruned");
        return Ok(result);
    }

    let lp = oracle.solve_lp()?;

    match lp {
        LpResult::Infeasible { farkas } => {
            println!(
                "  Node {node_id} (depth {depth}): LP infeasible → Farkas derivation"
            );
            let result = handle_infeasible(oracle, cert, &farkas, stats, eff_bounds)?;

            Ok(result)
        }
        LpResult::Optimal {
            obj_val,
            primals,
            duals,
            ..
        } => {
            println!(
                "  Node {node_id} (depth {depth}): LP optimal, obj = {obj_val:.6}"
            );
            let result = handle_optimal(
                oracle, cert, instance, bound_map, z_star, depth, stats, max_nodes,
                obj_val, primals, duals, strong, eff_bounds, branching,
                active_asms,
            )?;

            Ok(result)
        }
        LpResult::Unbounded => bail!("LP unbounded at B&B node (depth {depth})"),
    }
}

/// Handle an infeasible LP node: derive absurdity from Farkas certificate.
/// If Farkas derivation fails with dual simplex multipliers, retries with primal simplex.
fn handle_infeasible(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    farkas: &BTreeMap<ConIndex, f64>,
    stats: &mut RunStats,
    eff_bounds: &EffBounds,
) -> Result<NodeResult> {
    let node_bm = eff_bound_map(eff_bounds);
    // Include bound_map ConIndex values in extra so LHS repair can look up
    // branch-assumption constraints that aren't in the Farkas multiplier keys.
    let bm_indices = node_bm.values()
        .flat_map(|(lb, ub)| lb.iter().chain(ub.iter()).copied());
    let extra = build_extra(cert, farkas.keys().copied().chain(bm_indices));

    let farkas_result = match derive_farkas(farkas, cert.con_constraints(), &extra, &node_bm) {
        Ok(r) => r,
        Err(e) => {
            // Retry with primal simplex (Method=0) — different basis gives different Farkas multipliers.
            println!("    → Farkas failed ({e:#}), retrying with primal simplex...");
            let alt_lp = oracle.resolve_lp_with_method(0)?;
            match alt_lp {
                LpResult::Infeasible { farkas: alt_farkas } => {
                    let alt_bm_indices = node_bm.values()
                        .flat_map(|(lb, ub)| lb.iter().chain(ub.iter()).copied());
                    let alt_extra = build_extra(cert, alt_farkas.keys().copied().chain(alt_bm_indices));
                    derive_farkas(&alt_farkas, cert.con_constraints(), &alt_extra, &node_bm)
                        .map_err(|e2| anyhow::anyhow!(
                            "Farkas failed with both dual and primal simplex. \
                             Dual: {e:#}. Primal: {e2:#}"
                        ))?
                }
                _ => bail!(
                    "LP became non-infeasible with Method=0 (was infeasible with Method=1): {e:#}"
                ),
            }
        }
    };

    let assumptions = collect_assumptions(cert, &farkas_result.reason);
    let mut con = farkas_result.constraint;
    con.name = next_der_name(cert);

    stats.num_lin_steps += 1;
    let idx = cert.add_derivation(con, farkas_result.reason, assumptions);
    Ok(NodeResult::Absurdity(idx))
}

/// Handle an optimal LP node: derive bound, check pruning, or branch.
fn handle_optimal(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    depth: usize,
    stats: &mut RunStats,
    max_nodes: usize,
    obj_val: f64,
    primals: Vec<f64>,
    duals: BTreeMap<ConIndex, f64>,
    strong: bool,
    eff_bounds: &mut EffBounds,
    branching: &mut BranchingState,
    active_asms: &mut BTreeSet<ConIndex>,
) -> Result<NodeResult> {
    let node_bm = eff_bound_map(eff_bounds);
    let bm_indices = node_bm.values()
        .flat_map(|(lb, ub)| lb.iter().chain(ub.iter()).copied());
    let extra = build_extra(cert, duals.keys().copied().chain(bm_indices));

    // Check whether LP solution is integral (no fractional integer variable to branch on).
    let is_integral_normal = select_branch_var(&primals, &instance.variables, INTEGRALITY_TOL).is_none();

    // In infeasibility mode, an integral LP solution is a contradiction — the MIP
    // should be infeasible but we found a feasible integer point. Try tight tolerance
    // first: numerical near-integrality (e.g., 2.9999997) can fool the normal check.
    if z_star.is_none() && is_integral_normal {
        if select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL).is_none() {
            bail!(
                "LP solution is integral at depth {depth}, contradicting MIP infeasibility"
            );
        }
        println!("    → Near-integral LP in infeasibility mode, using tight-tolerance branching");
    }

    // Derive objective bound and check pruning (only when proving optimality).
    if let Some(z) = z_star {
        // Derive objective bound via cascaded rationalization (10^6..10^18).
        // When LP is integral, pass z* as pruning target so the cascade keeps trying
        // higher levels until the bound is tight enough (we can't branch as fallback).
        let pruning_target = if is_integral_normal { Some(z) } else { None };
        let mut derive_result = derive_bound(
            &duals,
            instance.obj_sense,
            &instance.obj_terms,
            cert.con_constraints(),
            &extra,
            &instance.variables,
            &node_bm,
            pruning_target,
        );

        // When LP appears integral but the bound doesn't meet z* (e.g., maximization
        // with LP bound = z*+1), try tight integrality tolerance. The LP can't truly be
        // integral at a value contradicting z*, so some variable must have a fractional
        // part below the normal 1e-6 tolerance.
        let is_integral = if is_integral_normal && derive_result.is_err() {
            if select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL).is_some() {
                println!("    → Near-integral LP, retrying with tight-tolerance branching");
                derive_result = derive_bound(
                    &duals,
                    instance.obj_sense,
                    &instance.obj_terms,
                    cert.con_constraints(),
                    &extra,
                    &instance.variables,
                    &node_bm,
                    None,
                );
                false
            } else {
                true
            }
        } else {
            is_integral_normal
        };

        // If bound derivation failed, check if we're stuck.
        if let Err(ref e) = derive_result {
            if is_integral {
                bail!(
                    "LP solution is integral but bound does not meet z* = {} (depth {depth}). {e:#}",
                    crate::vipr::fmt_rational(z)
                );
            }
            println!("    → Cascade failed ({e:#}), will branch");
        }
        if let Ok(dr) = derive_result {
            if dr.cascade_level == 0 {
                stats.quick_count += 1;
            } else {
                stats.cascade_count += 1;
            }

            let lin_assumptions = collect_assumptions(cert, &dr.reason);
            let mut lin_con = dr.constraint;
            lin_con.name = next_der_name(cert);

            stats.num_lin_steps += 1;
            let lin_idx = cert.add_derivation(lin_con.clone(), dr.reason, lin_assumptions);

            // Apply {rnd} if applicable.
            let (final_idx, final_rhs) = if dr.rnd_applicable {
                if let Some((mut rnd_con, rnd_reason)) = apply_rnd(&lin_con, lin_idx) {
                    rnd_con.name = next_der_name(cert);
                    let rnd_assumptions = cert.get_assumptions(lin_idx).cloned().unwrap_or_default();
                    stats.num_rnd_steps += 1;
                    let rnd_idx = cert.add_derivation(rnd_con.clone(), rnd_reason, rnd_assumptions);
                    (rnd_idx, rnd_con.rhs)
                } else {
                    (lin_idx, lin_con.rhs)
                }
            } else {
                (lin_idx, lin_con.rhs)
            };

            // Check pruning: derived bound meets z*?
            let pruned = match instance.obj_sense {
                ObjSense::Minimize => final_rhs >= *z,
                ObjSense::Maximize => final_rhs <= *z,
            };

            if pruned {
                println!(
                    "    → Pruned (bound {} meets z*)",
                    crate::vipr::fmt_rational(&final_rhs)
                );
                return Ok(NodeResult::Bound(final_idx));
            }
        }
    }

    // Not pruned (or infeasibility mode) — select branching variable.
    let branch = if strong {
        reliability_branch_select(oracle, &primals, &instance.variables, obj_val, branching)?
    } else {
        select_branch_var(&primals, &instance.variables, INTEGRALITY_TOL)
    };
    // Tight-tolerance fallback when normal branching finds nothing.
    let branch = branch.or_else(|| {
        let tight = select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL);
        if tight.is_some() {
            println!("    → Using tight-tolerance branching variable");
        }
        tight
    });

    let (branch_var, branch_val) = match branch {
        Some(pair) => pair,
        None => {
            if let Some(z) = z_star {
                bail!(
                    "LP solution is integral but bound does not meet z* = {} (depth {depth})",
                    crate::vipr::fmt_rational(z)
                )
            } else {
                bail!(
                    "LP solution is integral at depth {depth}, contradicting MIP infeasibility"
                )
            }
        },
    };

    let split_lo = crate::rational::floor(&crate::rational::rat_from_float(branch_val, 1_000_000));
    let split_hi = crate::rational::ceil(&crate::rational::rat_from_float(branch_val, 1_000_000));

    println!(
        "    → Branching on {} = {:.6} → x <= {} / x >= {} (depth {depth})",
        instance.variables[branch_var.0].name,
        branch_val,
        crate::vipr::fmt_rational(&split_lo),
        crate::vipr::fmt_rational(&split_hi),
    );

    // Branch: left (x <= floor(val)), right (x >= ceil(val)).
    branch_on(
        oracle, cert, instance, bound_map, z_star, branch_var, &split_lo, &split_hi, depth,
        stats, max_nodes, strong, eff_bounds, branching, active_asms,
    )
}

/// Branch on an integer variable: create {asm} constraints at split points, recurse, unsplit.
/// Left child: x <= split_lo (floor of fractional value).
/// Right child: x >= split_hi (ceil of fractional value).
fn branch_on(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    var: VarIndex,
    split_lo: &Rat,
    split_hi: &Rat,
    depth: usize,
    stats: &mut RunStats,
    max_nodes: usize,
    strong: bool,
    eff_bounds: &mut EffBounds,
    branching: &mut BranchingState,
    active_asms: &mut BTreeSet<ConIndex>,
) -> Result<NodeResult> {
    // --- Left child: x <= split_lo ---
    let left_asm_idx = cert.next_index();
    let mut left_terms = BTreeMap::new();
    left_terms.insert(var, rat_from_int(1));
    let left_asm_con = Constraint {
        name: next_der_name(cert),
        sense: Sense::Le,
        rhs: split_lo.clone(),
        terms: left_terms,
    };
    let left_asm_assumptions = BTreeSet::from([left_asm_idx]);
    stats.num_asm_steps += 1;
    let actual_left_asm = cert.add_derivation(
        left_asm_con.clone(),
        Reason::Asm,
        left_asm_assumptions,
    );
    debug_assert_eq!(actual_left_asm, left_asm_idx);

    let saved_left = eff_bounds.get(&var).cloned();
    apply_branch_to_eff_bounds(eff_bounds, var, Sense::Le, split_lo, left_asm_idx);
    oracle.push_branch_constraint(var, Sense::Le, split_lo, left_asm_idx)?;
    active_asms.insert(left_asm_idx);
    let left_result = solve_node(
        oracle, cert, instance, bound_map, z_star, depth + 1, stats, max_nodes, strong,
        eff_bounds, branching, active_asms,
    );
    active_asms.remove(&left_asm_idx);
    oracle.pop_branch_constraint()?;
    if let Some(old) = saved_left { eff_bounds.insert(var, old); }
    let left_result = left_result?;

    // Backjump: if left result doesn't depend on left_asm, it's valid at the parent.
    // Skip the right child entirely — the left result already proves the claim.
    {
        let left_idx = match left_result {
            NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
        };
        let left_asms = cert.get_assumptions(left_idx).cloned().unwrap_or_default();
        if !left_asms.contains(&left_asm_idx) {
            stats.num_backjumps += 1;
            println!("    → Backjump: left result independent of left assumption, skipping right child");
            return Ok(left_result);
        }
    }

    // --- Right child: x >= split_hi ---
    let right_asm_idx = cert.next_index();
    let mut right_terms = BTreeMap::new();
    right_terms.insert(var, rat_from_int(1));
    let right_asm_con = Constraint {
        name: next_der_name(cert),
        sense: Sense::Ge,
        rhs: split_hi.clone(),
        terms: right_terms,
    };
    let right_asm_assumptions = BTreeSet::from([right_asm_idx]);
    stats.num_asm_steps += 1;
    let actual_right_asm = cert.add_derivation(
        right_asm_con.clone(),
        Reason::Asm,
        right_asm_assumptions,
    );
    debug_assert_eq!(actual_right_asm, right_asm_idx);

    let saved_right = eff_bounds.get(&var).cloned();
    apply_branch_to_eff_bounds(eff_bounds, var, Sense::Ge, split_hi, right_asm_idx);
    oracle.push_branch_constraint(var, Sense::Ge, split_hi, right_asm_idx)?;
    active_asms.insert(right_asm_idx);
    let right_result = solve_node(
        oracle, cert, instance, bound_map, z_star, depth + 1, stats, max_nodes, strong,
        eff_bounds, branching, active_asms,
    );
    active_asms.remove(&right_asm_idx);
    oracle.pop_branch_constraint()?;
    if let Some(old) = saved_right { eff_bounds.insert(var, old); }
    let right_result = right_result?;

    // Backjump: if right result doesn't depend on right_asm, skip unsplit.
    // This enables cascading backjumps at parent levels.
    {
        let right_idx = match right_result {
            NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
        };
        let right_asms = cert.get_assumptions(right_idx).cloned().unwrap_or_default();
        if !right_asms.contains(&right_asm_idx) {
            stats.num_backjumps += 1;
            println!("    → Backjump: right result independent of right assumption");
            return Ok(right_result);
        }
    }

    // --- Unsplit ---
    let left_result_idx = match left_result {
        NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
    };
    let right_result_idx = match right_result {
        NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
    };

    // Compute unsplit assumptions.
    let left_asms = cert
        .get_assumptions(left_result_idx)
        .cloned()
        .unwrap_or_default();
    let right_asms = cert
        .get_assumptions(right_result_idx)
        .cloned()
        .unwrap_or_default();

    let mut uns_assumptions: BTreeSet<ConIndex> = BTreeSet::new();
    for &a in &left_asms {
        if a != left_asm_idx {
            uns_assumptions.insert(a);
        }
    }
    for &a in &right_asms {
        if a != right_asm_idx {
            uns_assumptions.insert(a);
        }
    }

    // Unsplit target: OBJ >= z* (or OBJ <= z* for max) when proving optimality,
    // or absurdity (0 >= 1) when proving infeasibility.
    let uns_con = if let Some(z) = z_star {
        let target_sense = match instance.obj_sense {
            ObjSense::Minimize => Sense::Ge,
            ObjSense::Maximize => Sense::Le,
        };
        Constraint {
            name: next_der_name(cert),
            sense: target_sense,
            rhs: z.clone(),
            terms: instance.obj_terms.clone(),
        }
    } else {
        absurdity_constraint(cert)
    };
    let uns_reason = Reason::Uns {
        left_result: left_result_idx,
        left_asm: left_asm_idx,
        right_result: right_result_idx,
        right_asm: right_asm_idx,
    };

    stats.num_uns_steps += 1;
    let uns_idx = cert.add_derivation(uns_con, uns_reason, uns_assumptions);
    if z_star.is_some() {
        Ok(NodeResult::Bound(uns_idx))
    } else {
        Ok(NodeResult::Absurdity(uns_idx))
    }
}

/// Select the most-fractional integer variable for branching.
/// Returns the variable index and its LP value (needed for floor/ceil split points).
fn select_branch_var(
    primals: &[f64],
    variables: &[crate::types::Variable],
    tol: f64,
) -> Option<(VarIndex, f64)> {
    let mut best: Option<(VarIndex, f64, f64)> = None; // (var, val, frac_part)
    for (i, var) in variables.iter().enumerate() {
        if !var.is_integer {
            continue;
        }
        let val = primals[i];
        let frac = (val - val.round()).abs();
        if frac > tol
            && best.as_ref().is_none_or(|b| frac > b.2)
        {
            best = Some((VarIndex(i), val, frac));
        }
    }
    best.map(|(vi, val, _)| (vi, val))
}

/// Reliability branching: strong-branch unreliable variables, pseudocost for reliable ones.
/// Combines the quality of strong branching with the speed of pseudocost branching.
fn reliability_branch_select(
    oracle: &mut Oracle,
    primals: &[f64],
    variables: &[crate::types::Variable],
    parent_obj: f64,
    state: &mut BranchingState,
) -> Result<Option<(VarIndex, f64)>> {
    // Collect fractional integer candidates.
    let mut candidates: Vec<(VarIndex, f64, f64)> = Vec::new(); // (var, val, frac)
    for (i, var) in variables.iter().enumerate() {
        if !var.is_integer {
            continue;
        }
        let val = primals[i];
        let frac = (val - val.round()).abs();
        if frac > INTEGRALITY_TOL {
            candidates.push((VarIndex(i), val, frac));
        }
    }

    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() == 1 {
        return Ok(Some((candidates[0].0, candidates[0].1)));
    }

    // Separate reliable vs unreliable candidates.
    let mut unreliable: Vec<(VarIndex, f64, f64)> = Vec::new();
    for &(vi, val, frac) in &candidates {
        let entry = state.pseudocosts.get(&vi);
        let is_reliable = entry.map_or(false, |e| {
            e.down_cnt >= RELIABILITY_THRESHOLD && e.up_cnt >= RELIABILITY_THRESHOLD
        });
        if !is_reliable {
            unreliable.push((vi, val, frac));
        }
    }

    // Strong-branch up to MAX_STRONG_CANDIDATES unreliable variables.
    // Sort by fractionality (most fractional first) to prioritize informative candidates.
    unreliable.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let n_strong = unreliable.len().min(MAX_STRONG_CANDIDATES);
    let dummy_idx = ConIndex(usize::MAX);

    for i in 0..n_strong {
        let (var_idx, val, _frac) = unreliable[i];
        let floor_val = crate::rational::floor(&crate::rational::rat_from_float(val, 1_000_000));
        let ceil_val = crate::rational::ceil(&crate::rational::rat_from_float(val, 1_000_000));
        let frac_part = val - val.floor();

        // Down branch: x <= floor(val)
        oracle.push_branch_constraint(var_idx, Sense::Le, &floor_val, dummy_idx)?;
        let down_lp = oracle.solve_lp()?;
        oracle.pop_branch_constraint()?;

        let down_obj = match &down_lp {
            LpResult::Optimal { obj_val, .. } => *obj_val,
            LpResult::Infeasible { .. } | LpResult::Unbounded => {
                return Ok(Some((var_idx, val)));
            }
        };

        // Up branch: x >= ceil(val)
        oracle.push_branch_constraint(var_idx, Sense::Ge, &ceil_val, dummy_idx)?;
        let up_lp = oracle.solve_lp()?;
        oracle.pop_branch_constraint()?;

        let up_obj = match &up_lp {
            LpResult::Optimal { obj_val, .. } => *obj_val,
            LpResult::Infeasible { .. } | LpResult::Unbounded => {
                return Ok(Some((var_idx, val)));
            }
        };

        // Update pseudocosts.
        let d_down = (down_obj - parent_obj).abs();
        let d_up = (up_obj - parent_obj).abs();
        let entry = state.pseudocosts.entry(var_idx).or_default();
        if frac_part > 1e-12 {
            entry.down_sum += d_down / frac_part;
            entry.down_cnt += 1;
        }
        if (1.0 - frac_part) > 1e-12 {
            entry.up_sum += d_up / (1.0 - frac_part);
            entry.up_cnt += 1;
        }
    }

    // Score all candidates using pseudocost estimates.
    let mut best_var = candidates[0].0;
    let mut best_val = candidates[0].1;
    let mut best_score = -1.0f64;

    for &(vi, val, _frac) in &candidates {
        let frac_part = val - val.floor();
        let entry = state.pseudocosts.get(&vi);
        let (q_down, q_up) = match entry {
            Some(e) if e.down_cnt > 0 && e.up_cnt > 0 => {
                let avg_down = e.down_sum / e.down_cnt as f64;
                let avg_up = e.up_sum / e.up_cnt as f64;
                (avg_down * frac_part, avg_up * (1.0 - frac_part))
            }
            _ => {
                // No pseudocost data: use fractionality as tie-breaker.
                let f = frac_part.min(1.0 - frac_part);
                (f, f)
            }
        };

        let score = (1.0 - RELIABILITY_MU) * q_down.min(q_up)
            + RELIABILITY_MU * q_down.max(q_up);

        if score > best_score {
            best_score = score;
            best_var = vi;
            best_val = val;
        }
    }

    Ok(Some((best_var, best_val)))
}

/// Collect assumptions from a Reason by unioning referenced constraints' assumption sets.
fn collect_assumptions(cert: &CertificateBuilder, reason: &Reason) -> BTreeSet<ConIndex> {
    match reason {
        Reason::Asm => unreachable!("Asm assumptions set directly by caller"),
        Reason::Lin(mults) | Reason::Rnd(mults) => {
            let mut asms = BTreeSet::new();
            for &(idx, _) in mults {
                if let Some(ref_asms) = cert.get_assumptions(idx) {
                    asms.extend(ref_asms);
                }
                // CON constraints: get_assumptions returns None → no assumptions.
            }
            asms
        }
        Reason::Uns {
            left_result,
            left_asm,
            right_result,
            right_asm,
        } => {
            let mut asms = BTreeSet::new();
            if let Some(left_asms) = cert.get_assumptions(*left_result) {
                for &a in left_asms {
                    if a != *left_asm {
                        asms.insert(a);
                    }
                }
            }
            if let Some(right_asms) = cert.get_assumptions(*right_result) {
                for &a in right_asms {
                    if a != *right_asm {
                        asms.insert(a);
                    }
                }
            }
            asms
        }
    }
}

/// Build extra constraint map for DER-indexed entries referenced in duals/farkas.
fn build_extra(
    cert: &CertificateBuilder,
    keys: impl Iterator<Item = ConIndex>,
) -> BTreeMap<ConIndex, Constraint> {
    let con_count = cert.con_count();
    let mut extra = BTreeMap::new();
    for idx in keys {
        if idx.0 >= con_count {
            extra.insert(idx, cert.get_constraint(idx).clone());
        }
    }
    extra
}

/// Build an absurdity constraint: 0 >= 1 (empty LHS, always false).
fn absurdity_constraint(cert: &CertificateBuilder) -> Constraint {
    Constraint {
        name: next_der_name(cert),
        sense: Sense::Ge,
        rhs: rat_from_int(1),
        terms: BTreeMap::new(),
    }
}

/// Generate next DER entry name: d0, d1, d2, ...
fn next_der_name(cert: &CertificateBuilder) -> String {
    let der_offset = cert.next_index().0 - cert.con_count();
    format!("d{}", der_offset)
}

// ===== Best-first B&B =====

/// Info about a single branch constraint in the path from root to node.
#[derive(Clone)]
struct BranchInfo {
    var: VarIndex,
    sense: Sense,
    rhs: Rat,
    asm_idx: ConIndex,
}

/// Which side of an unsplit this node represents.
#[derive(Clone, Copy)]
enum ChildSide {
    Left,
    Right,
}

/// A node waiting in the priority queue.
struct BfsNode {
    /// Complete path from root to this node.
    branches: Vec<BranchInfo>,
    /// LP primals at this node (for branch variable selection).
    primals: Vec<f64>,
    /// LP objective value at this node.
    obj_val: f64,
    /// Priority for BinaryHeap (higher = processed first).
    priority: f64,
    /// Index into the unsplit slots vector.
    parent_slot: usize,
    /// Which child of the parent slot this node is.
    parent_side: ChildSide,
}

impl PartialEq for BfsNode {
    fn eq(&self, other: &Self) -> bool {
        self.priority.total_cmp(&other.priority) == std::cmp::Ordering::Equal
    }
}

impl Eq for BfsNode {}

impl PartialOrd for BfsNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BfsNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.total_cmp(&other.priority)
    }
}

/// Tracks a pending {uns} derivation (waiting for both children to complete).
struct UnsplitSlot {
    left_asm: ConIndex,
    right_asm: ConIndex,
    left_result: Option<ConIndex>,
    right_result: Option<ConIndex>,
    parent_slot: Option<usize>,
    parent_side: Option<ChildSide>,
}

/// Outcome of trying to solve and derive at a child node.
enum ChildOutcome {
    /// Node was pruned (bound meets z* or Farkas absurdity).
    Pruned(ConIndex),
    /// Node needs branching — includes LP primals and objective for later use.
    NeedsBranching { primals: Vec<f64>, obj_val: f64 },
}

/// Compute priority for BinaryHeap ordering.
/// Higher priority = processed first. For min: higher bound is closer to z*.
/// For max: lower bound (negated) is closer to z*.
fn compute_priority(obj_val: f64, sense: ObjSense) -> f64 {
    match sense {
        ObjSense::Minimize => obj_val,
        ObjSense::Maximize => -obj_val,
    }
}

/// Solve the LP at the current oracle state and try to derive a bound.
/// Returns Pruned if the bound meets z*, NeedsBranching otherwise.
/// Increments node count and checks node limit.
fn try_solve_and_derive(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    z_star: Option<&Rat>,
    stats: &mut RunStats,
    max_nodes: usize,
    depth: usize,
    eff_bounds: &EffBounds,
    _active_asms: &BTreeSet<ConIndex>,
) -> Result<ChildOutcome> {
    stats.num_bnb_nodes += 1;
    if max_nodes > 0 && stats.num_bnb_nodes > max_nodes {
        bail!(
            "Node limit reached ({max_nodes}). Use --max-nodes to increase. \
             Current depth: {depth}, derivations: {}",
            cert.next_index().0 - cert.con_count()
        );
    }
    let node_id = stats.num_bnb_nodes;

    // CP pruning check (before LP solve).
    if let Some(result) = try_cp_pruning(cert, instance, eff_bounds, z_star, stats) {
        println!("  Node {node_id} (depth {depth}): CP pruned");
        let idx = match result {
            NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
        };
        return Ok(ChildOutcome::Pruned(idx));
    }

    let lp = oracle.solve_lp()?;

    match lp {
        LpResult::Infeasible { farkas } => {
            println!(
                "  Node {node_id} (depth {depth}): LP infeasible → Farkas derivation"
            );
            let result = handle_infeasible(oracle, cert, &farkas, stats, eff_bounds)?;

            let idx = match result {
                NodeResult::Bound(i) | NodeResult::Absurdity(i) => i,
            };
            Ok(ChildOutcome::Pruned(idx))
        }
        LpResult::Optimal {
            obj_val,
            primals,
            duals,
            ..
        } => {
            println!(
                "  Node {node_id} (depth {depth}): LP optimal, obj = {obj_val:.6}"
            );
            let node_bm = eff_bound_map(eff_bounds);
            let bm_indices = node_bm.values()
                .flat_map(|(lb, ub)| lb.iter().chain(ub.iter()).copied());
            let extra = build_extra(cert, duals.keys().copied().chain(bm_indices));
            let is_integral_normal = select_branch_var(&primals, &instance.variables, INTEGRALITY_TOL).is_none();

            // In infeasibility mode, an integral LP is a contradiction. Try tight
            // tolerance first: numerical near-integrality can fool the normal check.
            if z_star.is_none() && is_integral_normal {
                if select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL).is_none() {
                    bail!(
                        "LP solution is integral at depth {depth}, contradicting MIP infeasibility"
                    );
                }
                println!("    → Near-integral LP in infeasibility mode, using tight-tolerance branching");
            }

            // Derive objective bound and check pruning (only when proving optimality).
            if let Some(z) = z_star {
                let pruning_target = if is_integral_normal { Some(z) } else { None };

                let mut derive_result = derive_bound(
                    &duals,
                    instance.obj_sense,
                    &instance.obj_terms,
                    cert.con_constraints(),
                    &extra,
                    &instance.variables,
                    &node_bm,
                    pruning_target,
                );

                // Tight-tolerance fallback (same logic as handle_optimal).
                let is_integral = if is_integral_normal && derive_result.is_err() {
                    if select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL).is_some() {
                        println!("    → Near-integral LP, retrying with tight-tolerance branching");
                        derive_result = derive_bound(
                            &duals,
                            instance.obj_sense,
                            &instance.obj_terms,
                            cert.con_constraints(),
                            &extra,
                            &instance.variables,
                            &node_bm,
                            None,
                        );
                        false
                    } else {
                        true
                    }
                } else {
                    is_integral_normal
                };

                if let Err(ref e) = derive_result {
                    if is_integral {
                        bail!(
                            "LP solution is integral but bound does not meet z* = {} \
                             (depth {depth}). {e:#}",
                            crate::vipr::fmt_rational(z)
                        );
                    }
                    println!("    → Cascade failed ({e:#}), will branch");
                }

                if let Ok(dr) = derive_result {
                    if dr.cascade_level == 0 {
                        stats.quick_count += 1;
                    } else {
                        stats.cascade_count += 1;
                    }

                    let lin_assumptions = collect_assumptions(cert, &dr.reason);
                    let mut lin_con = dr.constraint;
                    lin_con.name = next_der_name(cert);
                    stats.num_lin_steps += 1;
                    let lin_idx =
                        cert.add_derivation(lin_con.clone(), dr.reason, lin_assumptions);

                    let (final_idx, final_rhs) = if dr.rnd_applicable {
                        if let Some((mut rnd_con, rnd_reason)) = apply_rnd(&lin_con, lin_idx)
                        {
                            rnd_con.name = next_der_name(cert);
                            let rnd_assumptions =
                                cert.get_assumptions(lin_idx).cloned().unwrap_or_default();
                            stats.num_rnd_steps += 1;
                            let rnd_idx = cert.add_derivation(
                                rnd_con.clone(),
                                rnd_reason,
                                rnd_assumptions,
                            );
                            (rnd_idx, rnd_con.rhs)
                        } else {
                            (lin_idx, lin_con.rhs)
                        }
                    } else {
                        (lin_idx, lin_con.rhs)
                    };

                    let pruned = match instance.obj_sense {
                        ObjSense::Minimize => final_rhs >= *z,
                        ObjSense::Maximize => final_rhs <= *z,
                    };

                    if pruned {
                        println!(
                            "    → Pruned (bound {} meets z*)",
                            crate::vipr::fmt_rational(&final_rhs)
                        );
                        return Ok(ChildOutcome::Pruned(final_idx));
                    }
                }

                if is_integral {
                    bail!(
                        "LP solution is integral but bound does not meet z* = {} \
                         (depth {depth})",
                        crate::vipr::fmt_rational(z)
                    );
                }
            }

            Ok(ChildOutcome::NeedsBranching { primals, obj_val })
        }
        LpResult::Unbounded => bail!("LP unbounded at B&B node (depth {depth})"),
    }
}

/// Process completed unsplit slots, cascading up the tree.
/// Returns Some(final_idx) if the root slot was completed, None otherwise.
fn cascade_unsplit(
    slots: &mut Vec<UnsplitSlot>,
    mut slot_id: usize,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    z_star: Option<&Rat>,
    stats: &mut RunStats,
) -> Option<ConIndex> {
    loop {
        let left_result = slots[slot_id].left_result.unwrap();
        let right_result = slots[slot_id].right_result.unwrap();
        let left_asm = slots[slot_id].left_asm;
        let right_asm = slots[slot_id].right_asm;

        // Compute unsplit assumptions.
        let left_asms = cert
            .get_assumptions(left_result)
            .cloned()
            .unwrap_or_default();
        let right_asms = cert
            .get_assumptions(right_result)
            .cloned()
            .unwrap_or_default();
        let mut uns_assumptions: BTreeSet<ConIndex> = BTreeSet::new();
        for &a in &left_asms {
            if a != left_asm {
                uns_assumptions.insert(a);
            }
        }
        for &a in &right_asms {
            if a != right_asm {
                uns_assumptions.insert(a);
            }
        }

        // Unsplit target: OBJ >= z* (or OBJ <= z* for max) when proving optimality,
        // or absurdity (0 >= 1) when proving infeasibility.
        let uns_con = if let Some(z) = z_star {
            let target_sense = match instance.obj_sense {
                ObjSense::Minimize => Sense::Ge,
                ObjSense::Maximize => Sense::Le,
            };
            Constraint {
                name: next_der_name(cert),
                sense: target_sense,
                rhs: z.clone(),
                terms: instance.obj_terms.clone(),
            }
        } else {
            absurdity_constraint(cert)
        };
        let uns_reason = Reason::Uns {
            left_result,
            left_asm,
            right_result,
            right_asm,
        };
        stats.num_uns_steps += 1;
        let uns_idx = cert.add_derivation(uns_con, uns_reason, uns_assumptions);

        // Record in parent slot and check for further cascade.
        let parent_slot = slots[slot_id].parent_slot;
        let parent_side = slots[slot_id].parent_side;
        if let (Some(pid), Some(side)) = (parent_slot, parent_side) {
            match side {
                ChildSide::Left => slots[pid].left_result = Some(uns_idx),
                ChildSide::Right => slots[pid].right_result = Some(uns_idx),
            }
            if slots[pid].left_result.is_some() && slots[pid].right_result.is_some() {
                slot_id = pid;
                continue; // cascade to parent
            }
            return None; // parent not yet complete
        }

        // No parent — this is the root slot.
        return Some(uns_idx);
    }
}

/// Run best-first B&B traversal using a priority queue.
/// Nodes with bounds closest to z* are processed first.
pub fn run_bnb_bestfs(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    max_nodes: usize,
    strong: bool,
    stats: &mut RunStats,
) -> Result<ConIndex> {
    let final_idx = run_bnb_bestfs_core(
        oracle, cert, instance, bound_map, z_star, max_nodes, strong, stats,
        BTreeSet::new(),
    )?;

    // Verify empty assumptions.
    if let Some(asms) = cert.get_assumptions(final_idx)
        && !asms.is_empty()
    {
        bail!(
            "Final derivation has non-empty assumptions: {:?}",
            asms
        );
    }

    Ok(final_idx)
}

/// Core best-first B&B traversal without empty-assumption assertion.
/// `initial_asms` are pre-existing active assumptions (e.g., from objective split).
fn run_bnb_bestfs_core(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: Option<&Rat>,
    max_nodes: usize,
    strong: bool,
    stats: &mut RunStats,
    initial_asms: BTreeSet<ConIndex>,
) -> Result<ConIndex> {
    let mut slots: Vec<UnsplitSlot> = Vec::new();
    let mut queue: BinaryHeap<BfsNode> = BinaryHeap::new();
    let mut branching = BranchingState { pseudocosts: BTreeMap::new() };
    let base_asms = initial_asms;

    // === Solve root node ===
    let root_eff = init_eff_bounds(instance, bound_map, cert);
    let root_outcome = try_solve_and_derive(
        oracle, cert, instance, z_star, stats, max_nodes, 0, &root_eff,
        &base_asms,
    )?;

    match root_outcome {
        ChildOutcome::Pruned(idx) => {
            println!("B&B complete: {} nodes explored", stats.num_bnb_nodes);
            return Ok(idx);
        }
        ChildOutcome::NeedsBranching { primals, obj_val } => {
            // Select branch variable at root.
            let branch = if strong {
                reliability_branch_select(oracle, &primals, &instance.variables, obj_val, &mut branching)?
            } else {
                select_branch_var(&primals, &instance.variables, INTEGRALITY_TOL)
            };
            let branch = branch.or_else(|| {
                let tight = select_branch_var(&primals, &instance.variables, TIGHT_INTEGRALITY_TOL);
                if tight.is_some() {
                    println!("    → Using tight-tolerance branching variable (root)");
                }
                tight
            });

            let (branch_var, branch_val) = match branch {
                Some(pair) => pair,
                None => {
                    if let Some(z) = z_star {
                        bail!(
                            "Root LP integral but bound does not meet z* = {}",
                            crate::vipr::fmt_rational(z)
                        )
                    } else {
                        bail!("Root LP integral, contradicting MIP infeasibility")
                    }
                }
            };

            let split_lo = crate::rational::floor(
                &crate::rational::rat_from_float(branch_val, 1_000_000),
            );
            let split_hi = crate::rational::ceil(
                &crate::rational::rat_from_float(branch_val, 1_000_000),
            );

            println!(
                "    → Branching on {} = {:.6} → x <= {} / x >= {} (depth 0)",
                instance.variables[branch_var.0].name,
                branch_val,
                crate::vipr::fmt_rational(&split_lo),
                crate::vipr::fmt_rational(&split_hi),
            );

            // Create {asm} entries for left and right branches.
            let left_asm_idx = cert.next_index();
            let mut left_terms = BTreeMap::new();
            left_terms.insert(branch_var, rat_from_int(1));
            let left_asm_con = Constraint {
                name: next_der_name(cert),
                sense: Sense::Le,
                rhs: split_lo.clone(),
                terms: left_terms,
            };
            stats.num_asm_steps += 1;
            cert.add_derivation(
                left_asm_con,
                Reason::Asm,
                BTreeSet::from([left_asm_idx]),
            );

            let right_asm_idx = cert.next_index();
            let mut right_terms = BTreeMap::new();
            right_terms.insert(branch_var, rat_from_int(1));
            let right_asm_con = Constraint {
                name: next_der_name(cert),
                sense: Sense::Ge,
                rhs: split_hi.clone(),
                terms: right_terms,
            };
            stats.num_asm_steps += 1;
            cert.add_derivation(
                right_asm_con,
                Reason::Asm,
                BTreeSet::from([right_asm_idx]),
            );

            // Create root unsplit slot (no parent).
            slots.push(UnsplitSlot {
                left_asm: left_asm_idx,
                right_asm: right_asm_idx,
                left_result: None,
                right_result: None,
                parent_slot: None,
                parent_side: None,
            });

            // Process left child.
            let left_eff = {
                let mut eff = root_eff.clone();
                apply_branch_to_eff_bounds(&mut eff, branch_var, Sense::Le, &split_lo, left_asm_idx);
                eff
            };
            oracle.push_branch_constraint(
                branch_var, Sense::Le, &split_lo, left_asm_idx,
            )?;
            let mut left_active = base_asms.clone();
            left_active.insert(left_asm_idx);
            let left_out = try_solve_and_derive(
                oracle, cert, instance, z_star, stats, max_nodes, 1, &left_eff,
                &left_active,
            )?;
            oracle.pop_branch_constraint()?;

            match left_out {
                ChildOutcome::Pruned(idx) => {
                    slots[0].left_result = Some(idx);
                }
                ChildOutcome::NeedsBranching {
                    primals: p,
                    obj_val: o,
                } => {
                    queue.push(BfsNode {
                        branches: vec![BranchInfo {
                            var: branch_var,
                            sense: Sense::Le,
                            rhs: split_lo.clone(),
                            asm_idx: left_asm_idx,
                        }],
                        primals: p,
                        obj_val: o,
                        priority: compute_priority(o, instance.obj_sense),
                        parent_slot: 0,
                        parent_side: ChildSide::Left,
                    });
                }
            }

            // Process right child.
            let right_eff = {
                let mut eff = root_eff;
                apply_branch_to_eff_bounds(&mut eff, branch_var, Sense::Ge, &split_hi, right_asm_idx);
                eff
            };
            oracle.push_branch_constraint(
                branch_var, Sense::Ge, &split_hi, right_asm_idx,
            )?;
            let mut right_active = base_asms.clone();
            right_active.insert(right_asm_idx);
            let right_out = try_solve_and_derive(
                oracle, cert, instance, z_star, stats, max_nodes, 1, &right_eff,
                &right_active,
            )?;
            oracle.pop_branch_constraint()?;

            match right_out {
                ChildOutcome::Pruned(idx) => {
                    slots[0].right_result = Some(idx);
                }
                ChildOutcome::NeedsBranching {
                    primals: p,
                    obj_val: o,
                } => {
                    queue.push(BfsNode {
                        branches: vec![BranchInfo {
                            var: branch_var,
                            sense: Sense::Ge,
                            rhs: split_hi.clone(),
                            asm_idx: right_asm_idx,
                        }],
                        primals: p,
                        obj_val: o,
                        priority: compute_priority(o, instance.obj_sense),
                        parent_slot: 0,
                        parent_side: ChildSide::Right,
                    });
                }
            }

            // Check if root slot is already complete (both children pruned).
            if slots[0].left_result.is_some() && slots[0].right_result.is_some() {
                if let Some(final_idx) =
                    cascade_unsplit(&mut slots, 0, cert, instance, z_star, stats)
                {
                    println!(
                        "B&B complete: {} nodes explored",
                        stats.num_bnb_nodes
                    );
                    return Ok(final_idx);
                }
            }
        }
    }

    // === Main loop: process queued nodes ===
    let mut max_queue_size = queue.len();
    while let Some(node) = queue.pop() {
        let depth = node.branches.len();
        let n_ancestors = depth;

        // Push all ancestor branch constraints to reconstruct oracle state (batch).
        oracle.push_branch_constraints_batch(
            node.branches.iter().map(|b| (b.var, b.sense, &b.rhs, b.asm_idx)),
        )?;

        // Compute effective bounds for this node.
        let node_eff = compute_eff_bounds_with_branches(instance, bound_map, cert, &node.branches);

        // Select branch variable using stored primals.
        let branch = if strong {
            reliability_branch_select(
                oracle,
                &node.primals,
                &instance.variables,
                node.obj_val,
                &mut branching,
            )?
        } else {
            select_branch_var(&node.primals, &instance.variables, INTEGRALITY_TOL)
        };
        let branch = branch.or_else(|| {
            let tight = select_branch_var(&node.primals, &instance.variables, TIGHT_INTEGRALITY_TOL);
            if tight.is_some() {
                println!("    → Using tight-tolerance branching variable (bestfs)");
            }
            tight
        });

        let (branch_var, branch_val) = match branch {
            Some(pair) => pair,
            None => {
                oracle.pop_n_branches(n_ancestors)?;
                if let Some(z) = z_star {
                    bail!(
                        "LP integral but bound does not meet z* = {} \
                         (bestfs, depth {depth})",
                        crate::vipr::fmt_rational(z)
                    );
                } else {
                    bail!(
                        "LP integral at depth {depth}, contradicting MIP infeasibility (bestfs)"
                    );
                }
            }
        };

        let split_lo = crate::rational::floor(
            &crate::rational::rat_from_float(branch_val, 1_000_000),
        );
        let split_hi = crate::rational::ceil(
            &crate::rational::rat_from_float(branch_val, 1_000_000),
        );

        println!(
            "    → Branching on {} = {:.6} → x <= {} / x >= {} (depth {depth})",
            instance.variables[branch_var.0].name,
            branch_val,
            crate::vipr::fmt_rational(&split_lo),
            crate::vipr::fmt_rational(&split_hi),
        );

        // Create {asm} entries.
        let left_asm_idx = cert.next_index();
        let mut left_terms = BTreeMap::new();
        left_terms.insert(branch_var, rat_from_int(1));
        let left_asm_con = Constraint {
            name: next_der_name(cert),
            sense: Sense::Le,
            rhs: split_lo.clone(),
            terms: left_terms,
        };
        stats.num_asm_steps += 1;
        cert.add_derivation(
            left_asm_con,
            Reason::Asm,
            BTreeSet::from([left_asm_idx]),
        );

        let right_asm_idx = cert.next_index();
        let mut right_terms = BTreeMap::new();
        right_terms.insert(branch_var, rat_from_int(1));
        let right_asm_con = Constraint {
            name: next_der_name(cert),
            sense: Sense::Ge,
            rhs: split_hi.clone(),
            terms: right_terms,
        };
        stats.num_asm_steps += 1;
        cert.add_derivation(
            right_asm_con,
            Reason::Asm,
            BTreeSet::from([right_asm_idx]),
        );

        // Create unsplit slot.
        let slot_id = slots.len();
        slots.push(UnsplitSlot {
            left_asm: left_asm_idx,
            right_asm: right_asm_idx,
            left_result: None,
            right_result: None,
            parent_slot: Some(node.parent_slot),
            parent_side: Some(node.parent_side),
        });

        // Process left child.
        let left_eff = {
            let mut eff = node_eff.clone();
            apply_branch_to_eff_bounds(&mut eff, branch_var, Sense::Le, &split_lo, left_asm_idx);
            eff
        };
        oracle.push_branch_constraint(
            branch_var, Sense::Le, &split_lo, left_asm_idx,
        )?;
        // Compute active assumptions for this node's children (include base_asms from obj split).
        let mut node_active: BTreeSet<ConIndex> = node.branches.iter().map(|b| b.asm_idx).collect();
        node_active.extend(&base_asms);

        let mut left_active = node_active.clone();
        left_active.insert(left_asm_idx);
        let left_out = try_solve_and_derive(
            oracle, cert, instance, z_star, stats, max_nodes,
            depth + 1, &left_eff, &left_active,
        )?;
        oracle.pop_branch_constraint()?;

        match left_out {
            ChildOutcome::Pruned(idx) => {
                slots[slot_id].left_result = Some(idx);
            }
            ChildOutcome::NeedsBranching {
                primals: p,
                obj_val: o,
            } => {
                let mut child_branches = node.branches.clone();
                child_branches.push(BranchInfo {
                    var: branch_var,
                    sense: Sense::Le,
                    rhs: split_lo.clone(),
                    asm_idx: left_asm_idx,
                });
                queue.push(BfsNode {
                    branches: child_branches,
                    primals: p,
                    obj_val: o,
                    priority: compute_priority(o, instance.obj_sense),
                    parent_slot: slot_id,
                    parent_side: ChildSide::Left,
                });
            }
        }

        // Process right child.
        let right_eff = {
            let mut eff = node_eff;
            apply_branch_to_eff_bounds(&mut eff, branch_var, Sense::Ge, &split_hi, right_asm_idx);
            eff
        };
        oracle.push_branch_constraint(
            branch_var, Sense::Ge, &split_hi, right_asm_idx,
        )?;
        let mut right_active = node_active;
        right_active.insert(right_asm_idx);
        let right_out = try_solve_and_derive(
            oracle, cert, instance, z_star, stats, max_nodes,
            depth + 1, &right_eff, &right_active,
        )?;
        oracle.pop_branch_constraint()?;

        match right_out {
            ChildOutcome::Pruned(idx) => {
                slots[slot_id].right_result = Some(idx);
            }
            ChildOutcome::NeedsBranching {
                primals: p,
                obj_val: o,
            } => {
                let mut child_branches = node.branches.clone();
                child_branches.push(BranchInfo {
                    var: branch_var,
                    sense: Sense::Ge,
                    rhs: split_hi.clone(),
                    asm_idx: right_asm_idx,
                });
                queue.push(BfsNode {
                    branches: child_branches,
                    primals: p,
                    obj_val: o,
                    priority: compute_priority(o, instance.obj_sense),
                    parent_slot: slot_id,
                    parent_side: ChildSide::Right,
                });
            }
        }

        // Pop all ancestor branches.
        oracle.pop_n_branches(n_ancestors)?;

        // Check if the new slot is complete and cascade.
        if slots[slot_id].left_result.is_some()
            && slots[slot_id].right_result.is_some()
        {
            if let Some(final_idx) =
                cascade_unsplit(&mut slots, slot_id, cert, instance, z_star, stats)
            {
                println!(
                    "B&B complete: {} nodes explored, max queue: {max_queue_size}",
                    stats.num_bnb_nodes
                );
                return Ok(final_idx);
            }
        }

        if queue.len() > max_queue_size {
            max_queue_size = queue.len();
        }
    }

    bail!(
        "B&B queue empty without completing root derivation \
         ({} nodes explored, {} slots)",
        stats.num_bnb_nodes,
        slots.len()
    )
}

// ===== Objective split =====

/// Run B&B with objective split at root when applicable.
///
/// When the objective is integer-valued (all non-zero coefficients are integers on
/// integer variables) and z* is integer, splits at root into:
///   - Left:  OBJ <= z*-1 (tighter LP relaxation → more pruning)
///   - Right: OBJ >= z*   (trivially the assumption itself for minimization)
/// Then unsplits to get OBJ >= z* with empty assumptions.
///
/// Falls through to normal B&B if objective split is not applicable.
pub fn run_bnb_with_obj_split(
    oracle: &mut Oracle,
    cert: &mut CertificateBuilder,
    instance: &Instance,
    bound_map: &BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
    z_star: &Rat,
    max_nodes: usize,
    strong: bool,
    bestfs: bool,
    stats: &mut RunStats,
) -> Result<ConIndex> {
    // Check applicability: objective must be integer-valued and z* must be integer.
    if !obj_integer_valued(instance) || !z_star.is_integer() {
        println!("Objective split: not applicable (obj not integer-valued or z* not integer)");
        return if bestfs {
            run_bnb_bestfs(oracle, cert, instance, bound_map, Some(z_star), max_nodes, strong, stats)
        } else {
            run_bnb(oracle, cert, instance, bound_map, Some(z_star), max_nodes, strong, stats)
        };
    }

    // Compute split points based on objective sense.
    let (left_sense, left_rhs, right_sense, right_rhs, target_sense) = match instance.obj_sense {
        ObjSense::Minimize => {
            // Left: OBJ <= z*-1, Right: OBJ >= z*, Target: OBJ >= z*
            (Sense::Le, z_star - rat_from_int(1), Sense::Ge, z_star.clone(), Sense::Ge)
        }
        ObjSense::Maximize => {
            // Left: OBJ >= z*+1, Right: OBJ <= z*, Target: OBJ <= z*
            (Sense::Ge, z_star + rat_from_int(1), Sense::Le, z_star.clone(), Sense::Le)
        }
    };

    println!(
        "Objective split: OBJ {} {} / OBJ {} {} (obj is integer-valued)",
        match left_sense { Sense::Le => "<=", Sense::Ge => ">=", Sense::Eq => "=" },
        crate::vipr::fmt_rational(&left_rhs),
        match right_sense { Sense::Le => "<=", Sense::Ge => ">=", Sense::Eq => "=" },
        crate::vipr::fmt_rational(&right_rhs),
    );

    // Create left {asm}: OBJ <sense> left_rhs
    let left_asm_idx = cert.next_index();
    let left_asm_con = Constraint {
        name: next_der_name(cert),
        sense: left_sense,
        rhs: left_rhs.clone(),
        terms: instance.obj_terms.clone(),
    };
    stats.num_asm_steps += 1;
    let actual_left = cert.add_derivation(
        left_asm_con,
        Reason::Asm,
        BTreeSet::from([left_asm_idx]),
    );
    debug_assert_eq!(actual_left, left_asm_idx);

    // Create right {asm}: OBJ <sense> right_rhs
    let right_asm_idx = cert.next_index();
    let right_asm_con = Constraint {
        name: next_der_name(cert),
        sense: right_sense,
        rhs: right_rhs.clone(),
        terms: instance.obj_terms.clone(),
    };
    stats.num_asm_steps += 1;
    let actual_right = cert.add_derivation(
        right_asm_con,
        Reason::Asm,
        BTreeSet::from([right_asm_idx]),
    );
    debug_assert_eq!(actual_right, right_asm_idx);

    // Push OBJ <= z*-1 (or OBJ >= z*+1 for max) to oracle.
    oracle.push_general_branch_constraint(&instance.obj_terms, left_sense, &left_rhs, left_asm_idx)?;

    // Run inner B&B with left_asm_idx as initial assumption.
    let left_initial = BTreeSet::from([left_asm_idx]);
    let left_result_idx = if bestfs {
        run_bnb_bestfs_core(
            oracle, cert, instance, bound_map, Some(z_star), max_nodes, strong, stats,
            left_initial,
        )?
    } else {
        run_bnb_core(
            oracle, cert, instance, bound_map, Some(z_star), max_nodes, strong, stats,
            left_initial,
        )?
    };

    // Pop the objective split constraint.
    oracle.pop_branch_constraint()?;

    // Right child is trivial: right_asm_idx itself is OBJ >= z* (or OBJ <= z* for max).
    let right_result_idx = right_asm_idx;

    // Compute unsplit assumptions.
    let left_asms = cert.get_assumptions(left_result_idx).cloned().unwrap_or_default();
    let right_asms = cert.get_assumptions(right_result_idx).cloned().unwrap_or_default();
    let mut uns_assumptions: BTreeSet<ConIndex> = BTreeSet::new();
    for &a in &left_asms {
        if a != left_asm_idx {
            uns_assumptions.insert(a);
        }
    }
    for &a in &right_asms {
        if a != right_asm_idx {
            uns_assumptions.insert(a);
        }
    }

    // Unsplit target: OBJ >= z* (min) or OBJ <= z* (max).
    let uns_con = Constraint {
        name: next_der_name(cert),
        sense: target_sense,
        rhs: z_star.clone(),
        terms: instance.obj_terms.clone(),
    };
    let uns_reason = Reason::Uns {
        left_result: left_result_idx,
        left_asm: left_asm_idx,
        right_result: right_result_idx,
        right_asm: right_asm_idx,
    };
    stats.num_uns_steps += 1;
    let final_idx = cert.add_derivation(uns_con, uns_reason, uns_assumptions.clone());

    // Verify empty assumptions.
    if !uns_assumptions.is_empty() {
        bail!(
            "Objective split final derivation has non-empty assumptions: {:?}",
            uns_assumptions
        );
    }

    stats.obj_split_used = true;
    println!("Objective split complete");
    Ok(final_idx)
}
