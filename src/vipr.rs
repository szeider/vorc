//! VIPR certificate builder and writer.
//!
//! Builds the certificate incrementally during B&B traversal,
//! then serializes to VIPR v1.1 format.

use crate::rational::Rat;
use crate::types::{ConIndex, ConPlan, Constraint, Derivation, Instance, Reason, VarIndex};
use anyhow::Result;
use num_bigint::BigInt;
use num_traits::{One, Zero};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

/// What the certificate proves.
pub enum RtpMode {
    /// Optimality: RTP range z z
    Range(Rat),
    /// Infeasibility: RTP infeas
    Infeas,
}

/// Builds a VIPR certificate incrementally.
pub struct CertificateBuilder {
    /// Original constraints (CON section), from the canonical ConPlan.
    con: Vec<Constraint>,
    /// How many CON entries are bound constraints (first n_bounds entries).
    n_bounds: usize,
    /// Derived constraints (DER section).
    der: Vec<Derivation>,
    /// Solutions (SOL section).
    solutions: Vec<(String, Vec<Rat>)>,
}

impl CertificateBuilder {
    /// Initialize from a canonical ConPlan (single source of truth for CON ordering).
    pub fn from_plan(plan: ConPlan) -> Self {
        let n_bounds = plan.n_bounds;
        CertificateBuilder {
            con: plan.constraints,
            n_bounds,
            der: Vec::new(),
            solutions: Vec::new(),
        }
    }

    /// Total constraint count (CON + DER so far).
    pub fn next_index(&self) -> ConIndex {
        ConIndex(self.con.len() + self.der.len())
    }

    /// Look up a constraint by its flat index (CON or DER).
    pub fn get_constraint(&self, idx: ConIndex) -> &Constraint {
        if idx.0 < self.con.len() {
            &self.con[idx.0]
        } else {
            &self.der[idx.0 - self.con.len()].constraint
        }
    }

    /// Look up a derivation by its flat index. Returns None for CON entries.
    pub fn get_derivation(&self, idx: ConIndex) -> Option<&Derivation> {
        if idx.0 < self.con.len() {
            None
        } else {
            self.der.get(idx.0 - self.con.len())
        }
    }

    /// Look up assumptions for a derivation. Returns None for CON entries.
    /// CON constraints have no assumptions (empty set implicitly).
    pub fn get_assumptions(&self, idx: ConIndex) -> Option<&BTreeSet<ConIndex>> {
        self.get_derivation(idx).map(|d| &d.assumptions)
    }

    /// Number of CON constraints.
    pub fn con_count(&self) -> usize {
        self.con.len()
    }


    /// Access CON constraints slice (for dual derivation).
    pub fn con_constraints(&self) -> &[Constraint] {
        &self.con
    }

    /// Add a derivation with explicit assumptions (computed by the caller).
    /// Returns the new derivation's ConIndex.
    pub fn add_derivation(
        &mut self,
        constraint: Constraint,
        reason: Reason,
        assumptions: BTreeSet<ConIndex>,
    ) -> ConIndex {
        let idx = self.next_index();
        self.der.push(Derivation {
            constraint,
            reason,
            assumptions,
        });
        idx
    }

    /// Add a named solution.
    pub fn add_solution(&mut self, name: String, values: Vec<Rat>) {
        self.solutions.push((name, values));
    }

    /// Print certificate statistics.
    pub fn print_stats(&self, instance: &Instance) {
        let n_vars = instance.variables.len();
        let n_int = instance.variables.iter().filter(|v| v.is_integer).count();
        let n_con = self.con.len();
        let n_der = self.der.len();

        let mut n_asm = 0usize;
        let mut n_lin = 0usize;
        let mut n_rnd = 0usize;
        let mut n_uns = 0usize;
        for d in &self.der {
            match &d.reason {
                Reason::Asm => n_asm += 1,
                Reason::Lin(_) => n_lin += 1,
                Reason::Rnd(_) => n_rnd += 1,
                Reason::Uns { .. } => n_uns += 1,
            }
        }

        eprintln!("\n--- Certificate Statistics ---");
        eprintln!("Variables:    {n_vars} ({n_int} integer)");
        eprintln!("Constraints:  {n_con}");
        eprintln!("Derivations:  {n_der}");
        eprintln!("  asm:        {n_asm}");
        eprintln!("  lin:        {n_lin}");
        eprintln!("  rnd:        {n_rnd}");
        eprintln!("  uns:        {n_uns}");
    }

    /// Write the complete certificate to a file.
    pub fn write(&self, path: &Path, instance: &Instance, rtp: &RtpMode) -> Result<()> {
        let mut f = std::fs::File::create(path)?;

        // Header comment.
        writeln!(f, "% VIPR certificate: oracle-guided B&B (Gurobi LP oracle)")?;

        // VER
        writeln!(f, "VER 1.1")?;

        // VAR
        let n_vars = instance.variables.len();
        writeln!(f, "VAR {}", n_vars)?;
        let var_names: Vec<&str> = instance.variables.iter().map(|v| v.name.as_str()).collect();
        writeln!(f, "{}", var_names.join(" "))?;

        // INT
        let int_indices: Vec<usize> = instance
            .variables
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_integer)
            .map(|(i, _)| i)
            .collect();
        writeln!(f, "INT {}", int_indices.len())?;
        if !int_indices.is_empty() {
            let idx_strs: Vec<String> = int_indices.iter().map(|i| i.to_string()).collect();
            writeln!(f, "{}", idx_strs.join(" "))?;
        }

        // OBJ
        let obj_sense_str = match instance.obj_sense {
            crate::types::ObjSense::Minimize => "min",
            crate::types::ObjSense::Maximize => "max",
        };
        writeln!(f, "OBJ {}", obj_sense_str)?;
        write_terms(&mut f, &instance.obj_terms)?;
        writeln!(f)?;

        // CON
        writeln!(f, "CON {} {}", self.con.len(), self.n_bounds)?;
        for con in &self.con {
            write_constraint(&mut f, con)?;
        }

        // RTP
        match rtp {
            RtpMode::Range(z) => writeln!(f, "RTP range {} {}", fmt_rational(z), fmt_rational(z))?,
            RtpMode::Infeas => writeln!(f, "RTP infeas")?,
        }

        // SOL
        writeln!(f, "SOL {}", self.solutions.len())?;
        for (name, values) in &self.solutions {
            // Count nonzero entries.
            let nonzero: Vec<(usize, &Rat)> = values
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.is_zero())
                .collect();
            write!(f, "{} {}", name, nonzero.len())?;
            for (idx, val) in &nonzero {
                write!(f, " {} {}", idx, fmt_rational(val))?;
            }
            writeln!(f)?;
        }

        // DER
        writeln!(f, "DER {}", self.der.len())?;
        for derivation in &self.der {
            write_derivation(&mut f, derivation, &instance.obj_terms)?;
        }

        Ok(())
    }
}

/// Write sparse terms: `n_terms idx1 coeff1 idx2 coeff2 ...`
fn write_terms(f: &mut impl Write, terms: &std::collections::BTreeMap<VarIndex, Rat>) -> Result<()> {
    write!(f, "{}", terms.len())?;
    for (var, coeff) in terms {
        write!(f, " {} {}", var.0, fmt_rational(coeff))?;
    }
    Ok(())
}

/// Write a constraint line: `name sense rhs n_terms idx1 coeff1 ...`
fn write_constraint(f: &mut impl Write, con: &Constraint) -> Result<()> {
    let sense_str = match con.sense {
        crate::types::Sense::Ge => "G",
        crate::types::Sense::Le => "L",
        crate::types::Sense::Eq => "E",
    };
    write!(f, "{} {} {} ", con.name, sense_str, fmt_rational(&con.rhs))?;
    write_terms(f, &con.terms)?;
    writeln!(f)?;
    Ok(())
}

/// Write a derivation line: `name sense rhs {OBJ|terms} {reason} last_ref`
fn write_derivation(
    f: &mut impl Write,
    der: &Derivation,
    obj_terms: &std::collections::BTreeMap<VarIndex, Rat>,
) -> Result<()> {
    let sense_str = match der.constraint.sense {
        crate::types::Sense::Ge => "G",
        crate::types::Sense::Le => "L",
        crate::types::Sense::Eq => "E",
    };

    write!(f, "{} {} {} ", der.constraint.name, sense_str, fmt_rational(&der.constraint.rhs))?;

    // Write LHS: OBJ shorthand if terms match objective, else explicit.
    if der.constraint.terms == *obj_terms {
        write!(f, "OBJ")?;
    } else {
        write_terms(f, &der.constraint.terms)?;
    }

    // Write reason.
    match &der.reason {
        Reason::Asm => {
            write!(f, " {{ asm }}")?;
        }
        Reason::Lin(mults) => {
            write!(f, " {{ lin {}", mults.len())?;
            for (idx, coeff) in mults {
                write!(f, " {} {}", idx.0, fmt_rational(coeff))?;
            }
            write!(f, " }}")?;
        }
        Reason::Rnd(mults) => {
            write!(f, " {{ rnd {}", mults.len())?;
            for (idx, coeff) in mults {
                write!(f, " {} {}", idx.0, fmt_rational(coeff))?;
            }
            write!(f, " }}")?;
        }
        Reason::Uns {
            left_result,
            left_asm,
            right_result,
            right_asm,
        } => {
            write!(
                f,
                " {{ uns {} {} {} {} }}",
                left_result.0, left_asm.0, right_result.0, right_asm.0
            )?;
        }
    }

    // last_ref: -1 (we don't compute cross-references).
    writeln!(f, " -1")?;

    Ok(())
}

/// Format a rational as VIPR value (integer or p/q fraction).
pub fn fmt_rational(v: &Rat) -> String {
    if v.denom() == &BigInt::one() {
        v.numer().to_string()
    } else {
        format!("{}/{}", v.numer(), v.denom())
    }
}

