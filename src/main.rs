mod bnb;
mod dual;
mod gomory;
mod mps;
mod oracle;
mod parser;
mod propagate;
mod rational;
mod types;
mod vipr;

use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::bnb::{run_bnb, run_bnb_bestfs, run_bnb_with_obj_split};
use crate::mps::parse_mps;
use crate::oracle::Oracle;
use crate::parser::parse_lp;
use crate::rational::{rat_from_float, Rat};
use crate::types::{ConPlan, Instance, MipResult, RunStats, Sense, Variable};
use crate::vipr::{CertificateBuilder, RtpMode};
use num_traits::Zero;

/// Denominator bound for rationalizing Gurobi float outputs (z*, x*).
const RATIONALIZE_DENOM: i64 = 1_000_000_000;

#[derive(Parser)]
#[command(name = "vipr-oracle", about = "Oracle-guided VIPR certificate constructor")]
struct Cli {
    /// Input instance (.lp or .mps/.mps.gz format)
    instance: PathBuf,

    /// Output certificate (.vipr)
    output: PathBuf,

    /// Use strong branching
    #[arg(long)]
    strong: bool,

    /// Print certificate statistics
    #[arg(long)]
    stats: bool,

    /// Maximum B&B nodes before aborting (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    max_nodes: usize,

    /// Enable bound propagation before B&B
    #[arg(long)]
    propagate: bool,

    /// Number of Gomory cut rounds (0 = disabled)
    #[arg(long, default_value_t = 0)]
    cuts: usize,

    /// Use best-first (priority queue) B&B instead of DFS
    #[arg(long)]
    bestfs: bool,

    /// Enable objective split at root (tighter LP relaxation when obj is integer-valued)
    #[arg(long)]
    rc_fix: bool,

    /// Set Gurobi NumericFocus globally (0=default, 1-3=increasing numeric care)
    #[arg(long, default_value_t = 0)]
    numeric_focus: i32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start_time = Instant::now();

    println!("vipr-oracle v{}", env!("CARGO_PKG_VERSION"));
    println!("Instance: {}", cli.instance.display());
    println!("Output:   {}", cli.output.display());

    // 1. Parse instance (dispatch on extension).
    let instance = {
        let path_str = cli.instance.to_string_lossy();
        if path_str.ends_with(".mps") || path_str.ends_with(".mps.gz") {
            parse_mps(&cli.instance)?
        } else {
            parse_lp(&cli.instance)?
        }
    };
    println!(
        "Parsed: {} variables, {} constraints",
        instance.variables.len(),
        instance.constraints.len()
    );

    // 2. Build canonical ConPlan.
    let plan = ConPlan::from_instance(&instance);
    println!(
        "ConPlan: {} rows ({} bounds + {} model)",
        plan.constraints.len(),
        plan.n_bounds,
        plan.constraints.len() - plan.n_bounds
    );

    // 3. Create oracle and solve MIP.
    let mut oracle = Oracle::new(&instance, &plan)?;
    if cli.numeric_focus > 0 {
        oracle.set_numeric_focus(cli.numeric_focus)?;
        println!("NumericFocus set to {}", cli.numeric_focus);
    }
    let mip_result = oracle.solve_mip()?;

    match mip_result {
        MipResult::Infeasible => {
            // --- Infeasibility certificate pipeline ---
            println!("MIP is infeasible — constructing infeasibility certificate.");

            // Permanently relax integrality — MIP solve is done, all further calls are LP.
            oracle.relax_permanently()?;

            // Initialize certificate builder.
            let mut bound_map = plan.bound_map.clone();
            let mut cert = CertificateBuilder::from_plan(plan);
            let mut stats = RunStats::default();

            // Bound propagation (before B&B).
            if cli.propagate {
                println!("Running bound propagation...");
                propagate::propagate_bounds(&mut cert, &mut oracle, &instance, &mut bound_map, &mut stats)?;
            }

            // Gomory cuts at root (before B&B).
            if cli.cuts > 0 {
                println!("Generating Gomory cuts ({} rounds)...", cli.cuts);
                gomory::generate_gomory_cuts(
                    &mut oracle, &mut cert, &instance, &bound_map, cli.cuts, &mut stats,
                )?;
            }

            // Run B&B with z_star = None (infeasibility mode).
            // Objective split is not applicable for infeasibility.
            if cli.rc_fix {
                println!("Warning: --rc-fix ignored for infeasibility certificates");
            }
            if cli.bestfs {
                println!("Warning: --bestfs has no useful priority signal for infeasibility (no z*)");
            }
            let mode = if cli.bestfs { "bestfs" } else { "DFS" };
            if cli.max_nodes > 0 {
                println!("Starting B&B ({mode}, max {} nodes, infeasibility)...", cli.max_nodes);
            } else {
                println!("Starting B&B ({mode}, unlimited nodes, infeasibility)...");
            }
            let bnb_result = if cli.bestfs {
                run_bnb_bestfs(
                    &mut oracle, &mut cert, &instance, &bound_map,
                    None, cli.max_nodes, cli.strong, &mut stats,
                )
            } else {
                run_bnb(
                    &mut oracle, &mut cert, &instance, &bound_map,
                    None, cli.max_nodes, cli.strong, &mut stats,
                )
            };
            if let Err(ref e) = bnb_result {
                eprintln!("B&B failed: {e:#}");
                print_stats_json(
                    &cli.instance, "FAIL", start_time.elapsed().as_secs_f64(),
                    &instance, None, &stats, oracle.lp_call_count(), 0,
                );
                return bnb_result.map(|_| ());
            }

            // No SOL section for infeasibility certificates.

            // Write certificate.
            cert.write(&cli.output, &instance, &RtpMode::Infeas)?;
            println!("Certificate written to {}", cli.output.display());

            if cli.stats {
                cert.print_stats(&instance);
            }

            // Print JSON summary.
            let cert_size_bytes = std::fs::metadata(&cli.output)
                .map(|m| m.len())
                .unwrap_or(0);
            print_stats_json(
                &cli.instance, "OK_INFEAS", start_time.elapsed().as_secs_f64(),
                &instance, None, &stats, oracle.lp_call_count(), cert_size_bytes,
            );
        }
        MipResult::Optimal { obj_val: z_star_f64, solution: x_star_f64 } => {
            // --- Optimality certificate pipeline ---
            let z_star = rat_from_float(z_star_f64, RATIONALIZE_DENOM);
            println!(
                "MIP optimal: z* = {z_star_f64} (rational: {})",
                vipr::fmt_rational(&z_star)
            );

            // Permanently relax integrality — MIP solve is done, all further calls are LP.
            oracle.relax_permanently()?;

            // Print nonzero solution values.
            println!("Solution:");
            for (i, &val) in x_star_f64.iter().enumerate() {
                if val.abs() > 1e-10 {
                    println!("  {} = {val}", instance.variables[i].name);
                }
            }

            // 4. Initialize certificate builder.
            let mut bound_map = plan.bound_map.clone();
            let mut cert = CertificateBuilder::from_plan(plan);
            let mut stats = RunStats::default();

            // 4a. Bound propagation (before B&B).
            if cli.propagate {
                println!("Running bound propagation...");
                propagate::propagate_bounds(&mut cert, &mut oracle, &instance, &mut bound_map, &mut stats)?;
            }

            // 4b. Gomory cuts at root (before B&B).
            if cli.cuts > 0 {
                println!("Generating Gomory cuts ({} rounds)...", cli.cuts);
                gomory::generate_gomory_cuts(
                    &mut oracle, &mut cert, &instance, &bound_map, cli.cuts, &mut stats,
                )?;
            }

            // 5. Run B&B (handles root-pruned and branching cases).
            let mode = if cli.bestfs { "bestfs" } else { "DFS" };
            let extra = if cli.rc_fix { " +obj-split" } else { "" };
            if cli.max_nodes > 0 {
                println!("Starting B&B ({mode}{extra}, max {} nodes)...", cli.max_nodes);
            } else {
                println!("Starting B&B ({mode}{extra}, unlimited nodes)...");
            }
            let bnb_result = if cli.rc_fix {
                run_bnb_with_obj_split(
                    &mut oracle,
                    &mut cert,
                    &instance,
                    &bound_map,
                    &z_star,
                    cli.max_nodes,
                    cli.strong,
                    cli.bestfs,
                    &mut stats,
                )
            } else if cli.bestfs {
                run_bnb_bestfs(
                    &mut oracle,
                    &mut cert,
                    &instance,
                    &bound_map,
                    Some(&z_star),
                    cli.max_nodes,
                    cli.strong,
                    &mut stats,
                )
            } else {
                run_bnb(
                    &mut oracle,
                    &mut cert,
                    &instance,
                    &bound_map,
                    Some(&z_star),
                    cli.max_nodes,
                    cli.strong,
                    &mut stats,
                )
            };
            if let Err(ref e) = bnb_result {
                eprintln!("B&B failed: {e:#}");
                print_stats_json(
                    &cli.instance, "FAIL", start_time.elapsed().as_secs_f64(),
                    &instance, Some(&z_star), &stats, oracle.lp_call_count(), 0,
                );
                return bnb_result.map(|_| ());
            }

            // 6. Repair and add solution.
            // Re-solve LP with integer vars fixed to avoid tiny numerical violations.
            let x_star_repaired = oracle.repair_solution(&x_star_f64, &instance.variables)?;
            let mut x_star_rat: Vec<Rat> = snap_solution(
                &x_star_repaired,
                &instance.variables,
            );

            // Repair solution to satisfy model constraints in exact arithmetic.
            let violations = repair_solution_exact(&mut x_star_rat, &instance);
            if violations > 0 {
                eprintln!("SOL repair failed: {violations} model constraints still violated");
                print_stats_json(
                    &cli.instance, "SOL_FAIL", start_time.elapsed().as_secs_f64(),
                    &instance, Some(&z_star), &stats, oracle.lp_call_count(), 0,
                );
                anyhow::bail!("SOL section has {violations} constraint violations (equality-coupled continuous variables)");
            }

            cert.add_solution("opt".to_string(), x_star_rat);

            // 7. Write certificate.
            cert.write(&cli.output, &instance, &RtpMode::Range(z_star.clone()))?;
            println!("Certificate written to {}", cli.output.display());

            if cli.stats {
                cert.print_stats(&instance);
            }

            // 8. Print JSON summary.
            let cert_size_bytes = std::fs::metadata(&cli.output)
                .map(|m| m.len())
                .unwrap_or(0);
            print_stats_json(
                &cli.instance, "OK", start_time.elapsed().as_secs_f64(),
                &instance, Some(&z_star), &stats, oracle.lp_call_count(), cert_size_bytes,
            );
        }
    }

    Ok(())
}

/// Get peak resident set size in MB via getrusage (macOS/Linux).
fn get_peak_memory_mb() -> f64 {
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        let mut usage = MaybeUninit::<libc::rusage>::uninit();
        let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if ret == 0 {
            let usage = unsafe { usage.assume_init() };
            let maxrss = usage.ru_maxrss;
            // macOS: bytes, Linux: kilobytes
            if cfg!(target_os = "macos") {
                maxrss as f64 / (1024.0 * 1024.0)
            } else {
                maxrss as f64 / 1024.0
            }
        } else {
            0.0
        }
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

/// Print STATS_JSON summary line. Called on all exit paths (OK, INFEASIBLE, FAIL).
fn print_stats_json(
    instance_path: &Path,
    status: &str,
    wall_time_s: f64,
    instance: &Instance,
    z_star: Option<&Rat>,
    stats: &RunStats,
    num_lp_calls: usize,
    cert_size_bytes: u64,
) {
    let raw_name = instance_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let instance_name = json_escape(&raw_name);
    let optimal_value = z_star
        .map(|z| vipr::fmt_rational(z))
        .unwrap_or_default();
    let peak_memory_mb = get_peak_memory_mb();

    println!(
        "STATS_JSON: {{\
         \"instance\":\"{instance_name}\",\
         \"status\":\"{status}\",\
         \"wall_time_s\":{wall_time_s:.2},\
         \"num_variables\":{},\
         \"num_constraints\":{},\
         \"optimal_value\":\"{}\",\
         \"num_bnb_nodes\":{},\
         \"num_lp_calls\":{num_lp_calls},\
         \"num_lin_steps\":{},\
         \"num_rnd_steps\":{},\
         \"num_asm_steps\":{},\
         \"num_uns_steps\":{},\
         \"quick_count\":{},\
         \"cascade_count\":{},\
         \"num_propagated_bounds\":{},\
         \"num_gomory_cuts\":{},\
         \"num_cp_domain_pruned\":{},\
         \"num_cp_bound_pruned\":{},\
         \"num_backjumps\":{},\
         \"num_cache_hits\":{},\
         \"num_rc_fixed\":{},\
         \"obj_split_used\":{},\
         \"cert_size_bytes\":{cert_size_bytes},\
         \"peak_memory_mb\":{peak_memory_mb:.1}\
         }}",
        instance.variables.len(),
        instance.constraints.len(),
        json_escape(&optimal_value),
        stats.num_bnb_nodes,
        stats.num_lin_steps,
        stats.num_rnd_steps,
        stats.num_asm_steps,
        stats.num_uns_steps,
        stats.quick_count,
        stats.cascade_count,
        stats.num_propagated_bounds,
        stats.num_gomory_cuts,
        stats.num_cp_domain_pruned,
        stats.num_cp_bound_pruned,
        stats.num_backjumps,
        stats.num_cache_hits,
        stats.num_rc_fixed,
        stats.obj_split_used,
    );
}

/// Escape a string for safe JSON embedding (handles `"` and `\`).
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Repair a rationalized solution to satisfy model constraints in exact arithmetic.
///
/// Handles pure-integer and simple MIP cases. For MIPs with equality-coupled
/// continuous variables, exact feasibility repair is a known limitation —
/// fixing equalities can violate inequalities (shared variables), requiring
/// a full rational LP feasibility solver which is out of scope.
///
/// Returns the number of remaining violations (0 = success).
fn repair_solution_exact(
    x: &mut Vec<Rat>,
    instance: &Instance,
) -> usize {
    // Check for continuous variables in equalities (the hard case).
    let has_eq_continuous = instance.constraints.iter().any(|con| {
        con.sense == Sense::Eq
            && con.terms.keys().any(|vi| !instance.variables[vi.0].is_integer)
    });

    let cv = count_all_violations(x, instance);
    let bv = count_bound_violations(x, &instance.variables);

    if cv == 0 && bv == 0 {
        return 0;
    }

    if has_eq_continuous && cv > 0 {
        eprintln!(
            "  SOL repair: {} constraint violations, {} bound violations",
            cv, bv
        );
        eprintln!(
            "  Note: exact SOL repair for MIPs with equality-coupled continuous \
             variables is not yet supported"
        );
    }

    cv + bv
}

/// Count violated model constraints.
fn count_all_violations(x: &[Rat], instance: &Instance) -> usize {
    let mut violations = 0;
    for con in &instance.constraints {
        let mut lhs = Rat::zero();
        for (vi, coeff) in &con.terms {
            lhs += coeff * &x[vi.0];
        }
        let ok = match con.sense {
            Sense::Eq => lhs == con.rhs,
            Sense::Le => lhs <= con.rhs,
            Sense::Ge => lhs >= con.rhs,
        };
        if !ok {
            violations += 1;
        }
    }
    violations
}

fn count_bound_violations(x: &[Rat], variables: &[Variable]) -> usize {
    let mut count = 0;
    for (i, var) in variables.iter().enumerate() {
        if let Some(ref lb) = var.lb {
            if x[i] < *lb {
                count += 1;
            }
        }
        if let Some(ref ub) = var.ub {
            if x[i] > *ub {
                count += 1;
            }
        }
    }
    count
}

/// Snap a Gurobi solution to exact rational values.
///
/// Strategy: round integers to nearest integer, convert continuous variables
/// using exact IEEE 754 conversion (zero approximation error), then clamp to bounds.
fn snap_solution(
    x_float: &[f64],
    variables: &[Variable],
) -> Vec<Rat> {
    x_float
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let var = &variables[i];

            if var.is_integer {
                // Round to nearest integer.
                rational::round(&rat_from_float(v, RATIONALIZE_DENOM))
            } else {
                // Use continued-fraction approximation with bounded denominator.
                // Exact IEEE 754 conversion preserves tiny floating-point noise
                // as huge-denominator rationals, causing SOL section failures.
                let mut r = rat_from_float(v, RATIONALIZE_DENOM);

                // Clamp to bounds.
                if let Some(ref lb) = var.lb {
                    if r < *lb {
                        r = lb.clone();
                    }
                }
                if let Some(ref ub) = var.ub {
                    if r > *ub {
                        r = ub.clone();
                    }
                }

                r
            }
        })
        .collect()
}
