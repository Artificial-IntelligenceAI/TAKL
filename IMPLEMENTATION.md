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
