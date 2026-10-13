# VORC: VIPR Oracle Construction

Rust implementation of oracle-guided VIPR certificate construction for
ILP/MIP, accompanying the paper:

> Stefan Szeider. *VIPR Certificate Construction from Black-Box ILP Solvers.*
> In Proceedings of the 32nd International Conference on Principles and
> Practice of Constraint Programming (CP 2026), LIPIcs, 2026.

VORC treats the ILP solver as a black-box LP oracle and translates its
floating-point dual multipliers into exact rational
[VIPR](https://github.com/scipopt/vipr) v1.1 certificates via
cascaded continued-fraction rationalization. The certificates are
self-contained and independently verifiable with `viprchk`; the
construction itself does not need an exact LP solver.

## Requirements

- Rust 1.85 or newer (edition 2024), installed via `rustup`
- Gurobi 12.0.3 (requires a valid Gurobi license; academic licenses are
  free for non-commercial use)
  - Set `GUROBI_HOME` to the Gurobi installation directory
  - Add `$GUROBI_HOME/lib` to `LD_LIBRARY_PATH` (Linux) or
    `DYLD_LIBRARY_PATH` (macOS)
  - If the `grb` build script cannot auto-detect the Gurobi library
    name, set `GUROBI_LIBNAME=gurobi120` explicitly (always needed on
    macOS, where the build script only looks for `.so` files)
- A C toolchain (for the Gurobi FFI bindings)

The `vendor/grb/` subdirectory contains a locally patched version of
the [`grb`](https://github.com/ykrist/rust-grb) crate required for the
Gurobi 12 C API; the top-level `Cargo.toml` includes the corresponding
`[patch.crates-io]` entry, so no external fetch is required.

## Build

```
cargo build --release
```

The resulting binary is at `target/release/vipr-oracle`.

## Usage

```
vipr-oracle <instance.lp | instance.mps[.gz]> <output.vipr> [flags]
```

Flags (see `vipr-oracle --help` for the full list):

- `--strong` use strong branching
- `--propagate` run bound propagation before branch-and-bound
- `--bestfs` use best-first branch-and-bound instead of DFS
- `--stats` print certificate statistics
- `--max-nodes N` abort after N branch-and-bound nodes

The certificate is written in VIPR v1.1 format and can be verified with
the reference checker `viprchk` from the
[VIPR tool suite](https://github.com/scipopt/vipr).

The experiments reported in the paper use the default mode (no flags
beyond the instance and output path): dual simplex, presolve off, dual
reductions off, Farkas certificates enabled. These are set internally
by the tool.

## Repository layout

```
src/
  main.rs        CLI entry point
  types.rs       core data types
  rational.rs    BigRational alias and helpers
  parser.rs      LP-format parser
  mps.rs         MPS-format parser
  oracle.rs      Gurobi interface (model, solve, duals, branching)
  bnb.rs         branch-and-bound
  dual.rs        exact dual computation (cascaded rationalization)
  propagate.rs   bound propagation
  gomory.rs      Gomory cut generation (experimental)
  vipr.rs        VIPR certificate builder and serializer
vendor/grb/      locally patched Gurobi Rust bindings
test/instances/  LP test instances
Cargo.toml       crate manifest
Cargo.lock       pinned dependency versions
LICENSE          MIT license
```

## Tests

```
cargo test --release
```

The unit tests parse and solve a small subset of the bundled LP
instances. Gurobi must be installed for the tests to compile; without it the
build stops with "Unable to infer libname".

## Reproducing the paper's experiments

Experimental data (benchmark results, instance generators) are archived
on Zenodo: <https://doi.org/10.5281/zenodo.20057193>.

## Citation

```bibtex
@inproceedings{Szeider26vorc,
  author    = {Stefan Szeider},
  title     = {{VIPR} Certificate Construction from Black-Box {ILP} Solvers},
  booktitle = {Proceedings of the 32nd International Conference on Principles
               and Practice of Constraint Programming (CP 2026)},
  series    = {LIPIcs},
  year      = {2026}
}
```

## License

MIT License. See [LICENSE](LICENSE).

The vendored `grb` crate (in `vendor/grb/`) is © Yannik Rist and is
distributed under its own MIT license; see `vendor/grb/LICENSE`.
