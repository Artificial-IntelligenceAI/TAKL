# TAKL — Implementation rules

The rules for the code that implements TAKL: the oracle and the compiler. `DECISIONS.md` records each decision with the
owner's words; this file collects the rules, so new code can be checked against them. Any AI reads this file in full
before contributing code. When a rule changes, `DECISIONS.md` gets a new entry and this file is updated to match.

## Sessions

- **design claude**: the decisions, `DECISIONS.md`, the oracle (`oracle/`) and the golden tests (`tests/`). The
  design session builds the oracle because it knows the design best. Works in `/Users/ts/TAKL`, on `main`.
- **compiler claude**: the compiler, in `compiler/`. That covers the bytecode, the interpreter that runs it, TAKL's
  IR, and Cranelift at both strengths. Works in its own clone, `/Users/ts/TAKL compiler`, on the same `main` with no
  branch, and runs `git pull --rebase` before pushing.

## Rules

1. **Language.** Rust only, Cranelift only. Nothing else without the owner's OK.
2. **Where.** compiler claude never edits `DECISIONS.md`, `IMPLEMENTATION.md`, `oracle/` or `tests/`.
3. **The oracle stays apart.** compiler claude reads the oracle's tests, never `oracle/src`. If the compiler copied
   the oracle, their bugs would agree.
4. **No design in code.** Nothing undecided is guessed. It gives a `NotDesignedYet` error, and the question goes to
   the owner through design claude.
5. **Tests.** Every golden test passes on the interpreter, on Cranelift forced to its quick strength, and on
   Cranelift forced to its speed strength, with output matching the oracle byte for byte. Nothing is pushed while a
   test fails.
6. **A kid's program never crashes TAKL.** No `unwrap`, `expect` or panic on a path real programs take. If TAKL
   itself fails, it says "TAKL broke, not you" and how to report it.
7. **Style.** `rustfmt`, `clippy`, and comments that say why.
8. **Commits.** `kind.folder.narrowing: sentence (owner: "their words")`. Kinds: `decision`, `design`, `feature`,
   `fix`, `test`, `house`. Each commit ends with its session's trailer: `Co-Authored-By: design claude
   <noreply@anthropic.com>` or `Co-Authored-By: compiler claude <noreply@anthropic.com>`.
9. **Cores.** No limit.

## Tracing

Every decision has a name: its commit's subject up to the colon, such as `decision.tables.no_holes`. The name is also
written under the decision's heading in `DECISIONS.md`. Code that carries out a decision says so with a comment:

```rust
// trace: decision.tables.no_holes
fn add_at(...) {
```

The oracle is tagged the same way the compiler is, so a decision can be followed from the log to the oracle to the compiler. A
check, written in Rust with the oracle, fails on a tag that names no decision, and lists the decisions nothing traces
yet.

## Naming

The code uses TAKL's own words, so the code, the decisions and the error messages all speak the same language.

- **Full words.** `expression`, not `expr`; `statement`, not `stmt`.
- **One name per concept.** A table is `table` everywhere.
- **TAKL's words over textbook words.** `WholeSize::Small`, not `I16`; `position_of`, not `index`; `Table`, not
  `Map` or `Vec`.
- **Rust's casing**, which `clippy` enforces: `SCREAMING_SNAKE_CASE` for constants (`TINY_WHOLE_BIGGEST`),
  `UpperCamelCase` for types, enums and traits (`SmallFraction`), `snake_case` for functions, variables, modules and
  files (`add_to_table`).
- **Tests are named after their decision.** `tests/tables/no_holes.takl` goes with `decision.tables.no_holes`.
- **Error kinds are named for what a kid sees.** `OutOfRange`, `NotDesignedYet`.
