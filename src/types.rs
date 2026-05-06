//! Core data types for VIPR certificate construction.

use crate::rational::{rat_from_int, Rat};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Newtype indices — prevent mixing variable, constraint, and Gurobi row indices
// ---------------------------------------------------------------------------

/// Index into the variable list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarIndex(pub usize);

/// Index into the flat constraint space (CON section + DER section).
/// 0-based internally; converted to VIPR format only in vipr.rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConIndex(pub usize);

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Constraint sense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sense {
    Ge, // >=
    Le, // <=
    Eq, // =
}

/// Objective sense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjSense {
    Minimize,
    Maximize,
}

// ---------------------------------------------------------------------------
// Core structures
// ---------------------------------------------------------------------------

/// A linear constraint: sense, rhs, and sparse LHS terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    pub name: String,
    pub sense: Sense,
    pub rhs: Rat,
    pub terms: BTreeMap<VarIndex, Rat>, // var_index -> coefficient
}

/// Variable metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub is_integer: bool,
    pub lb: Option<Rat>, // None = -inf
    pub ub: Option<Rat>, // None = +inf
}

/// A complete ILP/MIP instance.
#[derive(Debug)]
pub struct Instance {
    pub variables: Vec<Variable>,
    pub obj_sense: ObjSense,
    pub obj_terms: BTreeMap<VarIndex, Rat>,
    pub constraints: Vec<Constraint>,
}

// ---------------------------------------------------------------------------
// Derivation types
// ---------------------------------------------------------------------------

/// Reason for a derived constraint.
#[derive(Debug, Clone)]
pub enum Reason {
    /// Assumption (branching constraint).
    Asm,
    /// Linear combination with rational multipliers.
    Lin(Vec<(ConIndex, Rat)>),
    /// Rounded linear combination.
    Rnd(Vec<(ConIndex, Rat)>),
    /// Unsplit: merge two branches.
    Uns {
        left_result: ConIndex,
        left_asm: ConIndex,
        right_result: ConIndex,
        right_asm: ConIndex,
    },
}

/// A derived constraint in the DER section.
#[derive(Debug, Clone)]
pub struct Derivation {
    pub constraint: Constraint,
    pub reason: Reason,
    pub assumptions: BTreeSet<ConIndex>,
}

// ---------------------------------------------------------------------------
// Oracle result types
// ---------------------------------------------------------------------------

/// Result of solving an LP relaxation.
/// Duals/Farkas are sparse maps: missing key = zero dual.
/// Primals are indexed by VarIndex.0 (dense, one entry per variable).
#[derive(Debug)]
pub enum LpResult {
    Optimal {
        obj_val: f64,
        primals: Vec<f64>,
        duals: BTreeMap<ConIndex, f64>,
    },
    Infeasible {
        farkas: BTreeMap<ConIndex, f64>,
    },
    Unbounded,
}

/// Result of solving the MIP.
#[derive(Debug)]
pub enum MipResult {
    Optimal { obj_val: f64, solution: Vec<f64> },
    Infeasible,
}

/// What a B&B node produces.
#[derive(Debug)]
pub enum NodeResult {
    /// Derived an objective bound (index into DER).
    Bound(ConIndex),
    /// Derived an absurdity (index into DER).
    Absurdity(ConIndex),
}

// ---------------------------------------------------------------------------
// Run statistics — collected during B&B, printed as JSON at end
// ---------------------------------------------------------------------------

/// Statistics collected during a vipr-oracle run.
#[derive(Debug, Default)]
pub struct RunStats {
    pub num_bnb_nodes: usize,
    /// Nodes where rationalization succeeded at base level (max_denom = 10^6).
    pub quick_count: usize,
    /// Nodes where rationalization needed higher denominator bound (10^7..10^18).
    pub cascade_count: usize,
    pub num_lin_steps: usize,
    pub num_rnd_steps: usize,
    pub num_asm_steps: usize,
    pub num_uns_steps: usize,
    /// Number of bounds tightened by propagation.
    pub num_propagated_bounds: usize,
    /// Number of Gomory cuts added.
    pub num_gomory_cuts: usize,
    /// Nodes pruned by CP domain infeasibility (lb > ub).
    pub num_cp_domain_pruned: usize,
    /// Nodes pruned by CP objective bound from variable bounds.
    pub num_cp_bound_pruned: usize,
    /// Subtrees skipped by assumption-minimal backjumping.
    pub num_backjumps: usize,
    /// Nodes skipped by learned pruning cache (superset of previously-pruned assumptions).
    pub num_cache_hits: usize,
    /// Variables fixed by reduced-cost fixing.
    pub num_rc_fixed: usize,
    /// Whether objective split was applied at root.
    pub obj_split_used: bool,
}

// ---------------------------------------------------------------------------
// Canonical constraint plan — single source of truth for CON ordering
// ---------------------------------------------------------------------------

/// The definitive CON section: explicit bound rows + original constraints.
/// Built once from an Instance, then consumed by both CertificateBuilder and Oracle
/// to guarantee identical ConIndex assignments.
///
/// Canonical ordering (matches VIPR `CON m b` where first b entries are bounds):
///   [0..n_bounds)         — variable bounds, in VarIndex order:
///       for each variable: lb (G) if finite, then ub (L) if finite
///   [n_bounds..n_bounds + model_cons) — original model constraints
#[derive(Debug)]
pub struct ConPlan {
    /// All CON constraints in canonical order.
    pub constraints: Vec<Constraint>,
    /// How many are bound constraints (the first n_bounds entries).
    pub n_bounds: usize,
    /// Lookup: VarIndex → (Option<ConIndex> for lb, Option<ConIndex> for ub).
    pub bound_map: BTreeMap<VarIndex, (Option<ConIndex>, Option<ConIndex>)>,
}

impl ConPlan {
    /// Build the canonical CON plan from an Instance.
    /// Bounds come first (matching VIPR CON format), then model constraints.
    pub fn from_instance(instance: &Instance) -> Self {
        let mut constraints = Vec::new();
        let mut bound_map = BTreeMap::new();

        // Variable bounds first, in VarIndex order: lb then ub for each variable.
        for (i, var) in instance.variables.iter().enumerate() {
            let var_idx = VarIndex(i);
            let mut lb_idx = None;
            let mut ub_idx = None;

            if let Some(ref lb) = var.lb {
                let idx = ConIndex(constraints.len());
                let mut terms = BTreeMap::new();
                terms.insert(var_idx, rat_from_int(1));
                constraints.push(Constraint {
                    name: format!("lb{}", i),
                    sense: Sense::Ge,
                    rhs: lb.clone(),
                    terms,
                });
                lb_idx = Some(idx);
            }

            if let Some(ref ub) = var.ub {
                let idx = ConIndex(constraints.len());
                let mut terms = BTreeMap::new();
                terms.insert(var_idx, rat_from_int(1));
                constraints.push(Constraint {
                    name: format!("ub{}", i),
                    sense: Sense::Le,
                    rhs: ub.clone(),
                    terms,
                });
                ub_idx = Some(idx);
            }

            bound_map.insert(var_idx, (lb_idx, ub_idx));
        }
        let n_bounds = constraints.len();

        // Original model constraints after bounds.
        for c in &instance.constraints {
            constraints.push(c.clone());
        }

        ConPlan {
            constraints,
            n_bounds,
            bound_map,
        }
    }
}
