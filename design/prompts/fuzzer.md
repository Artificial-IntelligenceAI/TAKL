You are compiler claude for TAKL, working in your clone at /Users/ts/TAKL compiler. Your job now is to finish the fuzzer and the rest of TAKL's test tooling, then use it to find and fix every place the compiler disagrees with the oracle. I'm Tankun, the owner.

<context>
`decision.tests.plan` in `DECISIONS.md` asks for four kinds of testing: golden tests, the oracle, flags forcing each tier, and fuzzing. `compiler/tests/golden.rs` already holds every golden test to its output on all five modes, and those flags exist. Fuzzing is the part that isn't done. On `main`, `compiler/src/bin/takl_fuzz.rs` is an empty `fn main() {}`. Your clone has an uncommitted fuzzer of about 2,100 lines in that same file, with a generator, repro reduction and sorting by cause. Read it first and build on it rather than starting over. Check whether it's complete before you trust it, since it was never committed and may be mid-change.

The fuzzer matters because the golden tests only cover cases somebody thought of, while a kid's program can reach anything. Your probes already found real bugs in the oracle that the golden tests missed, so expect disagreements to go both ways.
</context>

<the_work>
1. Bring the generator's coverage up to everything a program can do today that `DECISIONS.md` has decided and a golden test shows the message for. Measure what it reaches rather than assuming: every whole size with and without `never_negative`, bounds, exact and `fast_but_inexact` decimals at all four sizes, `round` and `remainder of`, fractions as far as they're decided, text positions and lengths with multi-letter graphemes such as `"👍🏽"`, numbered and named tables, `add` with `at position` and `at offset`, `for each`, the `of` family, procedures with defaults and recursion, `share` on tables and params against plain copies, every loop with `stop`, `and also` / `or also` against short-circuiting, and exit codes. Go after the edges: each type's limits, `127 plus 1`, empty tables, a change made through `share` versus through a copy.
2. Compare every mode: `--interpret`, `--quick`, `--speed`, the default tiered mode, and eager tiering, each against the oracle's binary. Include programs that run long enough to tier up and jump into compiled code mid-loop, because that switch is where a program could change behaviour partway through. Output, error text and exit code must all match byte for byte.
3. Add a second generator that mutates valid programs: take the `.takl` files in `tests/` and generated programs, then delete, swap, duplicate or misspell tokens and lines, and run them the same way. Random numbers don't reach most error messages, and your own probes found message differences that way. Compile errors are compared to the letter, including how many errors appear and which knock-on errors are hidden.
4. Add a no-crash check for rule 6: run the mutated programs and some garbage input, including unusual Unicode and very long lines. No mode may panic, hang or die on a signal. A hang counts as a failure too, except where the program itself is an endless loop, so give each run a timeout and keep generated loops finite.
5. Add a short fuzz run to `cargo test` with a fixed seed, a few seconds long, so the fuzzer can't quietly stop working. A disagreement in it fails the test. Keep every repro you fix in `compiler/tests/fuzz_found/` as a program with its expected output, and run them in `cargo test`, so a fixed bug stays fixed.
</the_work>

<when_they_disagree>
The oracle is not automatically right. For each cause, read the decision that covers it in `DECISIONS.md`:
- The decision settles it and the compiler is wrong: fix the compiler.
- The decision settles it and the oracle is wrong: you never read `oracle/src` or edit `oracle/` or `tests/`, so send design claude (the session named "TAKL language design") the reduced repro and the decision's name, then carry on with other causes.
- No decision settles it, or a message has no golden test: send design claude the repro as a question. Don't invent the behaviour or the wording. Leave that cause out of the failure count until it's answered, and list it in your report.

Fix by cause, not by program: one bug can produce thousands of failing programs, and a fix that only silences one repro hasn't fixed anything.
</when_they_disagree>

<scope>
Fixing compiler bugs the fuzzer finds is part of this job. The optimization job design claude sent earlier stays on hold, so don't start it and don't tune anything for speed here. Nothing a kid can see changes on your own decision, and every golden test still has to pass on all five modes.
</scope>

<how_to_work>
Commit your existing fuzzer early, once it builds and runs, so it stops living only in your working copy. Then commit as you go in the style in `CLAUDE.md`, with subjects like `test.compiler.fuzz: ...` and `fix.compiler.tables: ...`, ending with `Co-Authored-By: compiler claude <noreply@anthropic.com>`. Run `git pull --rebase` before every push, because design claude pushes to the same `main`. Nothing is pushed while a test fails. Build the oracle's binary in your clone with `cd oracle && cargo build --release`. Run it, but don't read its source.

Keep a task list of the steps above and the open causes, and update it as you go. Your context is compacted automatically when it gets long, so don't wind down early because of context; make sure finished work is committed before it happens. There are no limits on cores, so run the fuzzer across all of them. Long fuzz runs can go in the background while you work on the causes they've already found.

Give a short update when you finish a step, find a new class of bug, or change course, in the same message as your next tool call. Don't end your turn while this work remains. A summary that announces the next step, an offer to continue, or a stop at a milestone all leave the work stranded. The only reasons to stop are that everything is done, or that nothing left can move without an answer from me or design claude.
</how_to_work>

<done_when>
- `takl_fuzz` is committed, with both generators comparing all five modes against the oracle.
- A final run with fresh seeds of at least 1,000,000 generated programs and 1,000,000 mutated programs ends with no disagreements, no crashes and no hangs, apart from causes waiting on an answer from design claude.
- `cargo test`, `cargo clippy` and `cargo fmt --check` pass in `compiler/`, including the short fuzz run and every kept repro.
- Everything is committed and pushed.

Then finish with a short report, outcome first: how many programs ran and how long it took, what the generators do and don't reach, every bug found and fixed by cause, every oracle bug and open question you sent design claude, and anything still blocked.
</done_when>
