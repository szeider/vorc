//! Gurobi LP oracle interface.
//!
//! Wraps Gurobi via the `grb` crate. Provides:
//! - Model construction from Instance + ConPlan
//! - MIP solve to get z* and x*
//! - LP solve with dual/Farkas extraction (v0.2+)
//! - LIFO push/pop of branching constraints as explicit rows (v0.3+)

use crate::rational::{rat_to_f64, Rat};
use crate::types::{ConIndex, ConPlan, Instance, LpResult, MipResult, ObjSense, Sense, VarIndex};
use anyhow::{bail, Result};
use grb::constr::IneqExpr;
use grb::prelude::*;
use grb::AsPtr;
use std::collections::BTreeMap;

/// Threshold below which dual/Farkas values are treated as zero.
const DUAL_ZERO_TOL: f64 = 1e-10;

/// The LP oracle backed by Gurobi.
pub struct Oracle {
    model: grb::Model,
    grb_vars: Vec<grb::Var>,
    grb_constrs: Vec<grb::Constr>,
    branch_stack: Vec<(grb::Constr, ConIndex)>,
    /// Permanent extra rows added by propagation or cutting planes.
    /// Unlike branch_stack, these are never removed.
    permanent_extra: Vec<(grb::Constr, ConIndex)>,
    lp_call_count: usize,
    /// If true, all VTypes are permanently set to Continuous (after MIP solve).
    /// solve_lp() skips per-call VType save/restore when this is set.
    is_relaxed: bool,
    /// User-configured NumericFocus level (0 = default). Used to restore after retry.
    numeric_focus: i32,
}

impl Oracle {
    /// Create oracle from an instance and canonical ConPlan.
    /// The ConPlan provides the definitive row ordering that matches
    /// the CertificateBuilder's ConIndex assignments.
    pub fn new(instance: &Instance, plan: &ConPlan) -> Result<Self> {
        let mut model = grb::Model::new("vipr")
            .map_err(|e| anyhow::anyhow!("Gurobi model creation failed: {e}"))?;

        // Set Gurobi parameters before adding anything.
        model
            .set_param(param::OutputFlag, 0)
            .map_err(|e| anyhow::anyhow!("set OutputFlag: {e}"))?;
        model
            .set_param(param::Method, 1)
            .map_err(|e| anyhow::anyhow!("set Method: {e}"))?;
        model
            .set_param(param::Presolve, 0)
            .map_err(|e| anyhow::anyhow!("set Presolve: {e}"))?;
        model
            .set_param(param::DualReductions, 0)
            .map_err(|e| anyhow::anyhow!("set DualReductions: {e}"))?;
        model
            .set_param(param::InfUnbdInfo, 1)
            .map_err(|e| anyhow::anyhow!("set InfUnbdInfo: {e}"))?;
        // Tighten tolerances to reduce borderline-integer values in MIP solutions.
        // Default IntFeasTol is 1e-5; tightening to 1e-9 ensures Gurobi reports
        // values much closer to exact integers, making post-solve rounding safer.
        model
            .set_param(param::IntFeasTol, 1e-9)
            .map_err(|e| anyhow::anyhow!("set IntFeasTol: {e}"))?;
        model
            .set_param(param::FeasibilityTol, 1e-9)
            .map_err(|e| anyhow::anyhow!("set FeasibilityTol: {e}"))?;
        model
            .set_param(param::OptimalityTol, 1e-9)
            .map_err(|e| anyhow::anyhow!("set OptimalityTol: {e}"))?;

        // Add variables with -inf/+inf bounds (actual bounds are ConPlan rows).
        // For MIP: set VarType, objective coefficient.
        let n_vars = instance.variables.len();
        let mut grb_vars = Vec::with_capacity(n_vars);

        for (i, var) in instance.variables.iter().enumerate() {
            let obj_coeff = instance
                .obj_terms
                .get(&VarIndex(i))
                .map(rat_to_f64)
                .unwrap_or(0.0);

            let vtype = if var.is_integer {
                // Check if binary: lb=0, ub=1
                let is_binary = var.lb.as_ref().map(rat_to_f64) == Some(0.0)
                    && var.ub.as_ref().map(rat_to_f64) == Some(1.0);
                if is_binary {
                    VarType::Binary
                } else {
                    VarType::Integer
                }
            } else {
                VarType::Continuous
            };

            let gvar = model
                .add_var(&var.name, vtype, obj_coeff, f64::NEG_INFINITY, f64::INFINITY, std::iter::empty())
                .map_err(|e| anyhow::anyhow!("add_var {}: {e}", var.name))?;
            grb_vars.push(gvar);
        }

        model
            .update()
            .map_err(|e| anyhow::anyhow!("model update after vars: {e}"))?;

        // Set objective sense.
        let sense = match instance.obj_sense {
            ObjSense::Minimize => ModelSense::Minimize,
            ObjSense::Maximize => ModelSense::Maximize,
        };
        model
            .set_attr(attr::ModelSense, sense)
            .map_err(|e| anyhow::anyhow!("set ModelSense: {e}"))?;

        // Add all ConPlan constraints as Gurobi rows.
        let mut grb_constrs = Vec::with_capacity(plan.constraints.len());

        for con in &plan.constraints {
            // Build LHS expression.
            let lhs: Expr = con
                .terms
                .iter()
                .map(|(vi, coeff)| Expr::Term(rat_to_f64(coeff), grb_vars[vi.0]))
                .fold(Expr::Constant(0.0), |acc, term| acc + term);

            let rhs = Expr::Constant(rat_to_f64(&con.rhs));

            let grb_sense = match con.sense {
                Sense::Le => ConstrSense::Less,
                Sense::Ge => ConstrSense::Greater,
                Sense::Eq => ConstrSense::Equal,
            };

            let ineq = IneqExpr {
                lhs,
                sense: grb_sense,
                rhs,
            };

            let gc = model
                .add_constr(&con.name, ineq)
                .map_err(|e| anyhow::anyhow!("add_constr {}: {e}", con.name))?;
            grb_constrs.push(gc);
        }

        model
            .update()
            .map_err(|e| anyhow::anyhow!("model update after constrs: {e}"))?;

        Ok(Oracle {
            model,
            grb_vars,
            grb_constrs,
            branch_stack: Vec::new(),
            permanent_extra: Vec::new(),
            lp_call_count: 0,
            is_relaxed: false,
            numeric_focus: 0,
        })
    }

    /// Set Gurobi NumericFocus parameter globally (0=default, 1-3=increasing care).
    /// Higher values make Gurobi more careful with numerical precision at the cost of speed.
    pub fn set_numeric_focus(&mut self, level: i32) -> Result<()> {
        self.model
            .set_param(param::NumericFocus, level)
            .map_err(|e| anyhow::anyhow!("set NumericFocus={level}: {e}"))?;
        self.numeric_focus = level;
        Ok(())
    }

    /// Solve the MIP to get claimed optimum z* and solution x*.
    ///
    /// Temporarily enables Gurobi presolve and auto method for the MIP solve,
    /// since we only need z* and x* here (not constraint-preserving duals).
    /// Restores LP-safe settings (Presolve=0, Method=1) after.
    pub fn solve_mip(&mut self) -> Result<MipResult> {
        // Enable aggressive settings for MIP solve.
        self.model.set_param(param::Presolve, -1)
            .map_err(|e| anyhow::anyhow!("set Presolve=-1: {e}"))?;
        self.model.set_param(param::Method, -1)
            .map_err(|e| anyhow::anyhow!("set Method=-1: {e}"))?;
        // Temporarily disable NumericFocus for MIP (only needed for LP duals).
        if self.numeric_focus > 0 {
            self.model.set_param(param::NumericFocus, 0)
                .map_err(|e| anyhow::anyhow!("set NumericFocus=0 for MIP: {e}"))?;
        }
        // Disable relative MIP gap — default 1e-4 can cause Gurobi to report
        // a suboptimal z* when the gap is tiny (e.g., 1 unit on value ~26000).
        // For certification we need the true optimum.
        self.model.set_param(param::MIPGap, 0.0)
            .map_err(|e| anyhow::anyhow!("set MIPGap=0: {e}"))?;

        self.model
            .optimize()
            .map_err(|e| anyhow::anyhow!("optimize: {e}"))?;

        let status = self
            .model
            .status()
            .map_err(|e| anyhow::anyhow!("status: {e}"))?;

        // Restore LP-safe settings.
        self.model.set_param(param::Presolve, 0)
            .map_err(|e| anyhow::anyhow!("restore Presolve=0: {e}"))?;
        self.model.set_param(param::Method, 1)
            .map_err(|e| anyhow::anyhow!("restore Method=1: {e}"))?;
        if self.numeric_focus > 0 {
            self.model.set_param(param::NumericFocus, self.numeric_focus)
                .map_err(|e| anyhow::anyhow!("restore NumericFocus={}: {e}", self.numeric_focus))?;
        }

        match status {
            Status::Optimal => {
                let obj_val: f64 = self
                    .model
                    .get_attr(attr::ObjVal)
                    .map_err(|e| anyhow::anyhow!("get ObjVal: {e}"))?;

                let x_vals: Vec<f64> = self
                    .model
                    .get_obj_attr_batch(attr::X, self.grb_vars.iter().copied())
                    .map_err(|e| anyhow::anyhow!("get X values: {e}"))?;

                Ok(MipResult::Optimal {
                    obj_val,
                    solution: x_vals,
                })
            }
            Status::Infeasible => Ok(MipResult::Infeasible),
            _ => bail!("MIP solve status: {status:?} (expected Optimal)"),
        }
    }

    /// Permanently relax all integer variables to continuous.
    /// Call after solve_mip() to eliminate per-LP VType toggling overhead.
    pub fn relax_permanently(&mut self) -> Result<()> {
        let continuous: Vec<(grb::Var, VarType)> = self
            .grb_vars
            .iter()
            .map(|&v| (v, VarType::Continuous))
            .collect();
        self.model
            .set_obj_attr_batch(attr::VType, continuous)
            .map_err(|e| anyhow::anyhow!("relax_permanently set VType: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("relax_permanently update: {e}"))?;
        self.is_relaxed = true;
        Ok(())
    }

    /// Number of LP solves performed so far.
    pub fn lp_call_count(&self) -> usize {
        self.lp_call_count
    }

    /// Solve LP relaxation at current node.
    /// If `is_relaxed` (after `relax_permanently()`), solves directly.
    /// Otherwise, temporarily relaxes integrality and restores after.
    pub fn solve_lp(&mut self) -> Result<LpResult> {
        self.lp_call_count += 1;

        if self.is_relaxed {
            return self.solve_lp_inner();
        }

        // Save original variable types.
        let orig_types: Vec<VarType> = self
            .model
            .get_obj_attr_batch(attr::VType, self.grb_vars.iter().copied())
            .map_err(|e| anyhow::anyhow!("get VType: {e}"))?;

        // Relax all to continuous.
        let continuous: Vec<(grb::Var, VarType)> = self
            .grb_vars
            .iter()
            .map(|&v| (v, VarType::Continuous))
            .collect();
        self.model
            .set_obj_attr_batch(attr::VType, continuous)
            .map_err(|e| anyhow::anyhow!("set VType to continuous: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after relax: {e}"))?;

        let result = self.solve_lp_inner();

        // Restore original variable types — always runs, even if solve/extract failed.
        let restore: Vec<(grb::Var, VarType)> = self
            .grb_vars
            .iter()
            .zip(orig_types.iter())
            .map(|(&v, &t)| (v, t))
            .collect();
        self.model
            .set_obj_attr_batch(attr::VType, restore)
            .map_err(|e| anyhow::anyhow!("restore VType: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after restore: {e}"))?;

        result
    }

    /// Inner LP solve: optimize + extract result. Assumes model is already relaxed.
    fn solve_lp_inner(&mut self) -> Result<LpResult> {
        self.model
            .optimize()
            .map_err(|e| anyhow::anyhow!("LP optimize: {e}"))?;

        let mut status = self
            .model
            .status()
            .map_err(|e| anyhow::anyhow!("LP status: {e}"))?;

        // Retry with NumericFocus=3 + Quad=1 on non-standard status (e.g. Numeric).
        if !matches!(status, Status::Optimal | Status::Infeasible | Status::Unbounded) {
            eprintln!("    LP status {status:?}, retrying with NumericFocus=3 Quad=1");
            self.model.set_param(param::NumericFocus, 3)
                .map_err(|e| anyhow::anyhow!("set NumericFocus=3: {e}"))?;
            self.model.set_param(param::Quad, 1)
                .map_err(|e| anyhow::anyhow!("set Quad=1: {e}"))?;
            self.model.optimize()
                .map_err(|e| anyhow::anyhow!("LP optimize (numeric retry): {e}"))?;
            status = self.model.status()
                .map_err(|e| anyhow::anyhow!("LP status (numeric retry): {e}"))?;
            // Restore to user-configured level.
            self.model.set_param(param::NumericFocus, self.numeric_focus)
                .map_err(|e| anyhow::anyhow!("restore NumericFocus={}: {e}", self.numeric_focus))?;
            self.model.set_param(param::Quad, -1)
                .map_err(|e| anyhow::anyhow!("restore Quad=-1: {e}"))?;
        }

        self.extract_lp_result(status)
    }

    /// Extract LP result from the current model state given the solve status.
    fn extract_lp_result(&self, status: Status) -> Result<LpResult> {
        match status {
            Status::Optimal => {
                let obj_val: f64 = self
                    .model
                    .get_attr(attr::ObjVal)
                    .map_err(|e| anyhow::anyhow!("get ObjVal: {e}"))?;

                let primals: Vec<f64> = self
                    .model
                    .get_obj_attr_batch(attr::X, self.grb_vars.iter().copied())
                    .map_err(|e| anyhow::anyhow!("get X: {e}"))?;

                let pi_vals: Vec<f64> = self
                    .model
                    .get_obj_attr_batch(attr::Pi, self.grb_constrs.iter().copied())
                    .map_err(|e| anyhow::anyhow!("get Pi: {e}"))?;

                // Build sparse dual map (skip near-zero).
                let mut duals = BTreeMap::new();
                for (i, &pi) in pi_vals.iter().enumerate() {
                    if pi.abs() > DUAL_ZERO_TOL {
                        duals.insert(ConIndex(i), pi);
                    }
                }

                // Include duals for permanent extra rows (propagation/cuts).
                for &(gc, con_idx) in &self.permanent_extra {
                    let perm_pi: Vec<f64> = self
                        .model
                        .get_obj_attr_batch(attr::Pi, std::iter::once(gc))
                        .map_err(|e| anyhow::anyhow!("get Pi for permanent: {e}"))?;
                    if perm_pi[0].abs() > DUAL_ZERO_TOL {
                        duals.insert(con_idx, perm_pi[0]);
                    }
                }

                // Include duals for active branch constraints.
                for &(gc, con_idx) in &self.branch_stack {
                    let branch_pi: Vec<f64> = self
                        .model
                        .get_obj_attr_batch(attr::Pi, std::iter::once(gc))
                        .map_err(|e| anyhow::anyhow!("get Pi for branch: {e}"))?;
                    if branch_pi[0].abs() > DUAL_ZERO_TOL {
                        duals.insert(con_idx, branch_pi[0]);
                    }
                }

                Ok(LpResult::Optimal {
                    obj_val,
                    primals,
                    duals,
                })
            }
            Status::Infeasible => {
                let farkas_vals: Vec<f64> = self
                    .model
                    .get_obj_attr_batch(attr::FarkasDual, self.grb_constrs.iter().copied())
                    .map_err(|e| anyhow::anyhow!("get FarkasDual: {e}"))?;

                let mut farkas = BTreeMap::new();
                for (i, &f) in farkas_vals.iter().enumerate() {
                    if f.abs() > DUAL_ZERO_TOL {
                        farkas.insert(ConIndex(i), f);
                    }
                }

                // Include Farkas for permanent extra rows.
                for &(gc, con_idx) in &self.permanent_extra {
                    let perm_farkas: Vec<f64> = self
                        .model
                        .get_obj_attr_batch(attr::FarkasDual, std::iter::once(gc))
                        .map_err(|e| anyhow::anyhow!("get FarkasDual for permanent: {e}"))?;
                    if perm_farkas[0].abs() > DUAL_ZERO_TOL {
                        farkas.insert(con_idx, perm_farkas[0]);
                    }
                }

                // Include Farkas for active branch constraints.
                for &(gc, con_idx) in &self.branch_stack {
                    let branch_farkas: Vec<f64> = self
                        .model
                        .get_obj_attr_batch(attr::FarkasDual, std::iter::once(gc))
                        .map_err(|e| anyhow::anyhow!("get FarkasDual for branch: {e}"))?;
                    if branch_farkas[0].abs() > DUAL_ZERO_TOL {
                        farkas.insert(con_idx, branch_farkas[0]);
                    }
                }

                Ok(LpResult::Infeasible { farkas })
            }
            Status::Unbounded => Ok(LpResult::Unbounded),
            _ => {
                anyhow::bail!("LP solve status: {status:?} (expected Optimal/Infeasible/Unbounded)");
            }
        }
    }

    /// Push a branching constraint as an explicit Gurobi row (LIFO).
    /// `con_idx` is the VIPR DER index already allocated by the caller.
    pub fn push_branch_constraint(
        &mut self,
        var: VarIndex,
        sense: Sense,
        rhs: &Rat,
        con_idx: ConIndex,
    ) -> Result<()> {
        let grb_sense = match sense {
            Sense::Le => ConstrSense::Less,
            Sense::Ge => ConstrSense::Greater,
            Sense::Eq => ConstrSense::Equal,
        };

        let lhs = Expr::Term(1.0, self.grb_vars[var.0]);
        let rhs_expr = Expr::Constant(rat_to_f64(rhs));

        let ineq = IneqExpr {
            lhs,
            sense: grb_sense,
            rhs: rhs_expr,
        };

        let name = format!("branch_{}", self.branch_stack.len());
        let gc = self
            .model
            .add_constr(&name, ineq)
            .map_err(|e| anyhow::anyhow!("add branch constraint: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after push: {e}"))?;

        self.branch_stack.push((gc, con_idx));
        Ok(())
    }

    /// Push a general (multi-variable) branching constraint as an explicit Gurobi row (LIFO).
    /// Used by objective split. `con_idx` is the VIPR DER index already allocated by the caller.
    pub fn push_general_branch_constraint(
        &mut self,
        terms: &BTreeMap<VarIndex, Rat>,
        sense: Sense,
        rhs: &Rat,
        con_idx: ConIndex,
    ) -> Result<()> {
        let grb_sense = match sense {
            Sense::Le => ConstrSense::Less,
            Sense::Ge => ConstrSense::Greater,
            Sense::Eq => ConstrSense::Equal,
        };

        let lhs: Expr = terms
            .iter()
            .map(|(vi, coeff)| Expr::Term(rat_to_f64(coeff), self.grb_vars[vi.0]))
            .fold(Expr::Constant(0.0), |acc, term| acc + term);
        let rhs_expr = Expr::Constant(rat_to_f64(rhs));

        let ineq = IneqExpr {
            lhs,
            sense: grb_sense,
            rhs: rhs_expr,
        };

        let name = format!("branch_{}", self.branch_stack.len());
        let gc = self
            .model
            .add_constr(&name, ineq)
            .map_err(|e| anyhow::anyhow!("add general branch constraint: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after general push: {e}"))?;

        self.branch_stack.push((gc, con_idx));
        Ok(())
    }

    /// Pop the most recent branching constraint (LIFO backtrack).
    pub fn pop_branch_constraint(&mut self) -> Result<()> {
        let (gc, _con_idx) = self
            .branch_stack
            .pop()
            .ok_or_else(|| anyhow::anyhow!("branch_stack empty on pop"))?;

        self.model
            .remove(gc)
            .map_err(|e| anyhow::anyhow!("remove branch constraint: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after pop: {e}"))?;

        Ok(())
    }

    /// Pop multiple branching constraints (LIFO backtrack), single model update.
    pub fn pop_n_branches(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            let (gc, _con_idx) = self
                .branch_stack
                .pop()
                .ok_or_else(|| anyhow::anyhow!("branch_stack empty on pop"))?;
            self.model
                .remove(gc)
                .map_err(|e| anyhow::anyhow!("remove branch constraint: {e}"))?;
        }
        if n > 0 {
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after batch pop: {e}"))?;
        }
        Ok(())
    }

    /// Push multiple branching constraints and call update() once.
    pub fn push_branch_constraints_batch<'a>(
        &mut self,
        items: impl IntoIterator<Item = (VarIndex, Sense, &'a Rat, ConIndex)>,
    ) -> Result<()> {
        let mut count = 0;
        for (var, sense, rhs, con_idx) in items {
            let grb_sense = match sense {
                Sense::Le => ConstrSense::Less,
                Sense::Ge => ConstrSense::Greater,
                Sense::Eq => ConstrSense::Equal,
            };
            let lhs = Expr::Term(1.0, self.grb_vars[var.0]);
            let rhs_expr = Expr::Constant(rat_to_f64(rhs));
            let ineq = IneqExpr {
                lhs,
                sense: grb_sense,
                rhs: rhs_expr,
            };
            let name = format!("branch_{}", self.branch_stack.len());
            let gc = self
                .model
                .add_constr(&name, ineq)
                .map_err(|e| anyhow::anyhow!("add branch constraint: {e}"))?;
            self.branch_stack.push((gc, con_idx));
            count += 1;
        }
        if count > 0 {
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after batch push: {e}"))?;
        }
        Ok(())
    }

    /// Re-solve LP with an alternative simplex method and return the result.
    /// Useful for getting different Farkas multipliers when the default method fails.
    pub fn resolve_lp_with_method(&mut self, method: i32) -> Result<LpResult> {
        self.model
            .set_param(param::Method, method)
            .map_err(|e| anyhow::anyhow!("set Method={method}: {e}"))?;
        let result = self.solve_lp();
        self.model
            .set_param(param::Method, 1) // restore dual simplex
            .map_err(|e| anyhow::anyhow!("restore Method=1: {e}"))?;
        result
    }

    /// Solve LP relaxation and also extract basis information (for Gomory cuts).
    /// Returns (LpResult, basis_head) where basis_head maps row position to variable index.
    pub fn solve_lp_with_basis(&mut self) -> Result<(LpResult, Vec<i32>)> {
        self.lp_call_count += 1;

        let needs_restore = !self.is_relaxed;
        let mut orig_types = Vec::new();
        if needs_restore {
            orig_types = self
                .model
                .get_obj_attr_batch(attr::VType, self.grb_vars.iter().copied())
                .map_err(|e| anyhow::anyhow!("get VType: {e}"))?;
            let continuous: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .map(|&v| (v, VarType::Continuous))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, continuous)
                .map_err(|e| anyhow::anyhow!("set VType to continuous: {e}"))?;
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after relax: {e}"))?;
        }

        // Solve LP and extract results + basis.
        let result = (|| -> Result<(LpResult, Vec<i32>)> {
            self.model
                .optimize()
                .map_err(|e| anyhow::anyhow!("LP optimize: {e}"))?;

            let status = self
                .model
                .status()
                .map_err(|e| anyhow::anyhow!("LP status: {e}"))?;

            let lp_result = self.extract_lp_result(status)?;

            // Get basis head while model is still relaxed.
            let total_rows = self.grb_constrs.len() + self.permanent_extra.len() + self.branch_stack.len();
            let basis_head = if matches!(status, Status::Optimal) {
                let mut bhead = vec![0i32; total_rows];
                let model_ptr = unsafe { self.model.as_mut_ptr() } as *mut grb_sys2::GRBmodel;
                let rc = unsafe { grb_sys2::GRBgetBasisHead(model_ptr, bhead.as_mut_ptr()) };
                if rc != 0 {
                    bail!("GRBgetBasisHead failed with error code {rc}");
                }
                bhead
            } else {
                Vec::new()
            };

            Ok((lp_result, basis_head))
        })();

        if needs_restore {
            let restore: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .zip(orig_types.iter())
                .map(|(&v, &t)| (v, t))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, restore)
                .map_err(|e| anyhow::anyhow!("restore VType: {e}"))?;
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after restore: {e}"))?;
        }

        result
    }

    /// Solve LP relaxation and also extract B^{-1} row for a specific basis position.
    /// Must be called on a freshly solved LP. Returns sparse (index, value) pairs.
    pub fn solve_lp_with_binv_row(&mut self, basis_pos: usize) -> Result<(LpResult, Vec<(usize, f64)>)> {
        self.lp_call_count += 1;

        let needs_restore = !self.is_relaxed;
        let mut orig_types = Vec::new();
        if needs_restore {
            orig_types = self
                .model
                .get_obj_attr_batch(attr::VType, self.grb_vars.iter().copied())
                .map_err(|e| anyhow::anyhow!("get VType: {e}"))?;
            let continuous: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .map(|&v| (v, VarType::Continuous))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, continuous)
                .map_err(|e| anyhow::anyhow!("set VType to continuous: {e}"))?;
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after relax: {e}"))?;
        }

        // Solve LP and extract results + binv row.
        let result = (|| -> Result<(LpResult, Vec<(usize, f64)>)> {
            self.model
                .optimize()
                .map_err(|e| anyhow::anyhow!("LP optimize: {e}"))?;

            let status = self
                .model
                .status()
                .map_err(|e| anyhow::anyhow!("LP status: {e}"))?;

            let lp_result = self.extract_lp_result(status)?;

            // Get binv row while model is still relaxed.
            let binv = if matches!(status, Status::Optimal) {
                let num_rows = self.grb_constrs.len() + self.permanent_extra.len() + self.branch_stack.len();
                let mut ind = vec![0i32; num_rows];
                let mut val = vec![0.0f64; num_rows];
                let mut svec = grb_sys2::GRBsvec {
                    len: 0,
                    ind: ind.as_mut_ptr(),
                    val: val.as_mut_ptr(),
                };
                let model_ptr = unsafe { self.model.as_mut_ptr() } as *mut grb_sys2::GRBmodel;
                let rc = unsafe { grb_sys2::GRBBinvRowi(model_ptr, basis_pos as i32, &mut svec) };
                if rc != 0 {
                    bail!("GRBBinvRowi({basis_pos}) failed with error code {rc}");
                }
                let len = svec.len as usize;
                (0..len).map(|k| (ind[k] as usize, val[k])).collect()
            } else {
                Vec::new()
            };

            Ok((lp_result, binv))
        })();

        if needs_restore {
            let restore: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .zip(orig_types.iter())
                .map(|(&v, &t)| (v, t))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, restore)
                .map_err(|e| anyhow::anyhow!("restore VType: {e}"))?;
            self.model
                .update()
                .map_err(|e| anyhow::anyhow!("update after restore: {e}"))?;
        }

        result
    }

    /// Number of Gurobi constraint rows (CON section only, no permanent/branch).
    pub fn num_con_rows(&self) -> usize {
        self.grb_constrs.len()
    }

    /// Add a permanent single-variable bound constraint to the model.
    /// Used by bound propagation. These rows stay for the entire solve.
    pub fn add_permanent_bound(
        &mut self,
        var: VarIndex,
        sense: Sense,
        rhs: &Rat,
        con_idx: ConIndex,
    ) -> Result<()> {
        let grb_sense = match sense {
            Sense::Le => ConstrSense::Less,
            Sense::Ge => ConstrSense::Greater,
            Sense::Eq => ConstrSense::Equal,
        };

        let lhs = Expr::Term(1.0, self.grb_vars[var.0]);
        let rhs_expr = Expr::Constant(rat_to_f64(rhs));

        let ineq = IneqExpr {
            lhs,
            sense: grb_sense,
            rhs: rhs_expr,
        };

        let name = format!("perm_bound_{}", self.permanent_extra.len());
        let gc = self
            .model
            .add_constr(&name, ineq)
            .map_err(|e| anyhow::anyhow!("add permanent bound: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after permanent bound: {e}"))?;

        self.permanent_extra.push((gc, con_idx));
        Ok(())
    }

    /// Add a permanent multi-variable cut constraint to the model.
    /// Used by Gomory cut generation. These rows stay for the entire solve.
    pub fn add_permanent_cut(
        &mut self,
        terms: &BTreeMap<VarIndex, Rat>,
        sense: Sense,
        rhs: &Rat,
        con_idx: ConIndex,
    ) -> Result<()> {
        let grb_sense = match sense {
            Sense::Le => ConstrSense::Less,
            Sense::Ge => ConstrSense::Greater,
            Sense::Eq => ConstrSense::Equal,
        };

        let lhs: Expr = terms
            .iter()
            .map(|(vi, coeff)| Expr::Term(rat_to_f64(coeff), self.grb_vars[vi.0]))
            .fold(Expr::Constant(0.0), |acc, term| acc + term);
        let rhs_expr = Expr::Constant(rat_to_f64(rhs));

        let ineq = IneqExpr {
            lhs,
            sense: grb_sense,
            rhs: rhs_expr,
        };

        let name = format!("perm_cut_{}", self.permanent_extra.len());
        let gc = self
            .model
            .add_constr(&name, ineq)
            .map_err(|e| anyhow::anyhow!("add permanent cut: {e}"))?;
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after permanent cut: {e}"))?;

        self.permanent_extra.push((gc, con_idx));
        Ok(())
    }

    /// Repair a MIP solution by fixing integer variables to rounded values
    /// and re-solving the LP to get feasible continuous values.
    ///
    /// This avoids tiny numerical violations (~1e-9) that Gurobi tolerates
    /// but viprchk rejects in exact rational arithmetic.
    pub fn repair_solution(
        &mut self,
        x_mip: &[f64],
        variables: &[crate::types::Variable],
    ) -> Result<Vec<f64>> {
        let needs_restore = !self.is_relaxed;
        let mut orig_types = Vec::new();
        if needs_restore {
            orig_types = self
                .model
                .get_obj_attr_batch(attr::VType, self.grb_vars.iter().copied())
                .map_err(|e| anyhow::anyhow!("get VType: {e}"))?;
            let continuous: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .map(|&v| (v, VarType::Continuous))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, continuous)
                .map_err(|e| anyhow::anyhow!("set VType to continuous: {e}"))?;
        }

        // Fix integer variables to their rounded MIP values via equality constraints.
        let mut fix_constrs: Vec<grb::Constr> = Vec::new();
        for (i, var) in variables.iter().enumerate() {
            if !var.is_integer {
                continue;
            }
            let rounded = x_mip[i].round();
            let lhs = Expr::Term(1.0, self.grb_vars[i]);
            let rhs = Expr::Constant(rounded);
            let ineq = IneqExpr {
                lhs,
                sense: ConstrSense::Equal,
                rhs,
            };
            let gc = self
                .model
                .add_constr(&format!("fix_{i}"), ineq)
                .map_err(|e| anyhow::anyhow!("add fix constraint for var {i}: {e}"))?;
            fix_constrs.push(gc);
        }

        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after fix: {e}"))?;

        // Solve LP.
        self.model
            .optimize()
            .map_err(|e| anyhow::anyhow!("repair LP optimize: {e}"))?;

        let status = self
            .model
            .status()
            .map_err(|e| anyhow::anyhow!("repair LP status: {e}"))?;

        let result = if status == Status::Optimal {
            let x_vals: Vec<f64> = self
                .model
                .get_obj_attr_batch(attr::X, self.grb_vars.iter().copied())
                .map_err(|e| anyhow::anyhow!("repair LP get X: {e}"))?;
            Ok(x_vals)
        } else {
            // LP infeasible with fixed integers — fall back to original MIP solution.
            eprintln!(
                "Warning: repair LP status {:?}, falling back to raw MIP solution",
                status
            );
            Ok(x_mip.to_vec())
        };

        // Remove fix constraints and restore variable types if needed.
        for gc in fix_constrs {
            self.model
                .remove(gc)
                .map_err(|e| anyhow::anyhow!("remove fix constraint: {e}"))?;
        }
        if needs_restore {
            let restore: Vec<(grb::Var, VarType)> = self
                .grb_vars
                .iter()
                .zip(orig_types.iter())
                .map(|(&v, &t)| (v, t))
                .collect();
            self.model
                .set_obj_attr_batch(attr::VType, restore)
                .map_err(|e| anyhow::anyhow!("restore VType: {e}"))?;
        }
        self.model
            .update()
            .map_err(|e| anyhow::anyhow!("update after repair cleanup: {e}"))?;

        result
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_lp;
    use crate::types::{ConPlan, LpResult};

    fn test_instances_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/instances")
    }

    #[test]
    fn test_solve_mip_tiny1() {
        let path = test_instances_dir().join("tiny1.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();
        let result = oracle.solve_mip().unwrap();
        let (z_star, x_star) = match result {
            MipResult::Optimal { obj_val, solution } => (obj_val, solution),
            _ => panic!("Expected Optimal"),
        };
        assert!(
            (z_star - 2.0).abs() < 1e-6,
            "tiny1 z* should be 2.0, got {z_star}"
        );
        // x=1, y=1
        assert!((x_star[0] - 1.0).abs() < 1e-6, "x should be 1");
        assert!((x_star[1] - 1.0).abs() < 1e-6, "y should be 1");
    }

    #[test]
    fn test_solve_mip_tiny2() {
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();
        let result = oracle.solve_mip().unwrap();
        let (z_star, _x_star) = match result {
            MipResult::Optimal { obj_val, solution } => (obj_val, solution),
            _ => panic!("Expected Optimal"),
        };
        assert!(
            (z_star - 4.0).abs() < 1e-6,
            "tiny2 z* should be 4.0, got {z_star}"
        );
    }

    #[test]
    fn test_solve_mip_genint4() {
        let path = test_instances_dir().join("genint4.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();
        let result = oracle.solve_mip().unwrap();
        let (z_star, x_star) = match result {
            MipResult::Optimal { obj_val, solution } => (obj_val, solution),
            _ => panic!("Expected Optimal"),
        };
        assert!(z_star.is_finite(), "genint4 z* should be finite");
        assert!(z_star > 0.0, "genint4 z* should be positive (maximize)");
        // All variables should be integer-valued.
        for (i, &val) in x_star.iter().enumerate() {
            assert!(
                (val - val.round()).abs() < 1e-6,
                "x{} should be integer, got {val}",
                i + 1
            );
        }
    }

    #[test]
    fn test_solve_lp_tiny1() {
        // tiny1: min x+y, LP opt = 4/3 (fractional)
        let path = test_instances_dir().join("tiny1.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();
        let result = oracle.solve_lp().unwrap();
        match result {
            LpResult::Optimal {
                obj_val, duals, ..
            } => {
                assert!(
                    (obj_val - 4.0 / 3.0).abs() < 1e-6,
                    "tiny1 LP opt should be 4/3, got {obj_val}"
                );
                assert!(!duals.is_empty(), "should have nonzero duals");
            }
            _ => panic!("Expected Optimal, got {result:?}"),
        }
        // Verify MIP still works after LP solve (types restored).
        let mip = oracle.solve_mip().unwrap();
        let z_star = match mip {
            MipResult::Optimal { obj_val, .. } => obj_val,
            _ => panic!("Expected Optimal"),
        };
        assert!(
            (z_star - 2.0).abs() < 1e-6,
            "MIP should still work after LP, got {z_star}"
        );
    }

    #[test]
    fn test_solve_lp_cg() {
        // cg: min x+y, LP opt = 1/4
        let path = test_instances_dir().join("cg.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();
        let result = oracle.solve_lp().unwrap();
        match result {
            LpResult::Optimal {
                obj_val, duals, ..
            } => {
                assert!(
                    (obj_val - 0.25).abs() < 1e-6,
                    "cg LP opt should be 1/4, got {obj_val}"
                );
                assert!(!duals.is_empty(), "should have nonzero duals");
            }
            _ => panic!("Expected Optimal, got {result:?}"),
        }
    }

    #[test]
    fn test_push_pop_basic() {
        // tiny2: max 3x+4y, 2x+3y<=4, binary x,y. LP opt=5.333
        // Push x<=0 → LP should change. Pop → LP should restore.
        use crate::rational::rat_from_int;
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        // Baseline LP.
        let base = oracle.solve_lp().unwrap();
        let base_obj = match &base {
            LpResult::Optimal { obj_val, .. } => *obj_val,
            _ => panic!("Expected Optimal"),
        };

        // Push x <= 0 (fix x=0 for binary). Use a dummy ConIndex.
        let rhs_zero = rat_from_int(0);
        oracle
            .push_branch_constraint(VarIndex(0), Sense::Le, &rhs_zero, ConIndex(100))
            .unwrap();

        // LP with x<=0: only y free, max 4y s.t. 3y<=4 → y=4/3, obj=16/3
        let after_push = oracle.solve_lp().unwrap();
        let push_obj = match &after_push {
            LpResult::Optimal { obj_val, .. } => *obj_val,
            _ => panic!("Expected Optimal after push"),
        };
        assert!(
            (push_obj - base_obj).abs() > 0.01,
            "LP should change after push: base={base_obj}, after={push_obj}"
        );

        // Pop → should restore original LP.
        oracle.pop_branch_constraint().unwrap();
        let after_pop = oracle.solve_lp().unwrap();
        let pop_obj = match &after_pop {
            LpResult::Optimal { obj_val, .. } => *obj_val,
            _ => panic!("Expected Optimal after pop"),
        };
        assert!(
            (pop_obj - base_obj).abs() < 1e-6,
            "LP should restore after pop: base={base_obj}, after={pop_obj}"
        );
    }

    #[test]
    fn test_push_makes_infeasible() {
        // tiny2: max 3x+4y, 2x+3y<=4, binary x,y.
        // Push x>=1 and y>=1 → 2+3=5>4 → infeasible (with bounds x<=1, y<=1 from binary).
        use crate::rational::rat_from_int;
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        let rhs_one = rat_from_int(1);
        oracle
            .push_branch_constraint(VarIndex(0), Sense::Ge, &rhs_one, ConIndex(100))
            .unwrap();
        oracle
            .push_branch_constraint(VarIndex(1), Sense::Ge, &rhs_one, ConIndex(101))
            .unwrap();

        let result = oracle.solve_lp().unwrap();
        match &result {
            LpResult::Infeasible { farkas } => {
                assert!(!farkas.is_empty(), "should have Farkas certificate");
            }
            _ => panic!("Expected Infeasible, got {result:?}"),
        }

        // Pop both, should restore feasible.
        oracle.pop_branch_constraint().unwrap();
        oracle.pop_branch_constraint().unwrap();
        let restored = oracle.solve_lp().unwrap();
        assert!(
            matches!(restored, LpResult::Optimal { .. }),
            "Should be Optimal after popping all"
        );
    }

    #[test]
    fn test_branch_duals_included() {
        // After pushing a binding branch constraint, its dual should appear in the map.
        use crate::rational::rat_from_int;
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        // Push x <= 0 (binding, since LP wants x > 0).
        let rhs_zero = rat_from_int(0);
        let branch_con_idx = ConIndex(100);
        oracle
            .push_branch_constraint(VarIndex(0), Sense::Le, &rhs_zero, branch_con_idx)
            .unwrap();

        let result = oracle.solve_lp().unwrap();
        match &result {
            LpResult::Optimal { duals, .. } => {
                // The branch constraint should have a nonzero dual (it's binding).
                assert!(
                    duals.contains_key(&branch_con_idx),
                    "branch constraint dual should be in map, got keys: {:?}",
                    duals.keys().collect::<Vec<_>>()
                );
            }
            _ => panic!("Expected Optimal"),
        }

        oracle.pop_branch_constraint().unwrap();
    }

    #[test]
    fn test_pop_empty_errors() {
        let path = test_instances_dir().join("tiny2.lp");
        let inst = parse_lp(&path).unwrap();
        let plan = ConPlan::from_instance(&inst);
        let mut oracle = Oracle::new(&inst, &plan).unwrap();

        // Pop on empty stack should error.
        assert!(oracle.pop_branch_constraint().is_err());
    }

}
