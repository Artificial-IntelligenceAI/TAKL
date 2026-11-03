# TAKL — Design decisions

Running log. Each entry: date, the decision, who proposed it, and who chose it.
Nothing here is final until the owner says so; open questions live at the bottom.

## Decided

### 2026-11-03 — `start_running` is required: every program runs from `procedure start_running:` (owner: "required")

`decision.procedures.start_running_required`

Asked whether a program may also begin at its first line, the owner: "required". A program with no `start_running` is a
compile error naming it.

### 2026-11-03 — Empty text and an empty table are written `empty`; `""` and `[]` are answered "did you mean `empty`" (owner: "yea")

`decision.values.empty`

```
let name: text = empty
let scores: table of small_whole = empty
```

The owner proposed `nothing` in place of `""`, "so it would look like it's deliberate". Design claude agreed it should
look deliberate but proposed another word: empty text is a value, text of no letters, while `nothing` is no value at
all, and one word for both lets `let x: text = greet "Sam"` look right. One name per concept gives `empty`, which serves
tables too. The owner: "Are both words accurate?" Yes, `nothing` being grammar only. The owner: "yea".

### 2026-11-03 — `return nothing` leaves a procedure that gives nothing back; `nothing` is grammar, never a value (owner: "yea")

`decision.procedures.return_nothing`

The owner: "Should it return void? Like `return nothing`?" Design claude: yes, and it says what comes back better than
a bare `return`. Asked whether the word is accurate, design claude: only if `nothing` is not a value, since
`let x: text = nothing` would rebuild Ruby's `nil`; so `nothing` appears only after `return`, never stored or handed
on. The owner: "yea".

### 2026-11-03 — A program can have `procedure start_running:` ... `end procedure start_running`, where running starts (owner)

`decision.procedures.start_running`

```
procedure start_running:
  show "Hello, World!"
end procedure start_running
```

The owner asked whether TAKL should have `fn main`, then: "if we have it. What would it be called?" Design claude
proposed `start`, other languages' `main` named in help. The owner wrote `start_running`, which pairs with the errors'
`Stopped before running:` and `Stopped while running:`; a name, so glued. It gives nothing back. Required: the entry above.

### 2026-11-03 — The compiler is built with its optimizations from the start, not added after a plain first version (owner)

`decision.house.optimize_early`

The owner: "Oh btw, the real product would have optimizations from start". The oracle stays plain; compiler claude
builds TAKL's IR passes and Cranelift's two strengths as it goes, never a slow version to be sped up later. Each
optimization is held to the oracle on the golden tests and the fuzzer from the day it lands, since an optimization is
where tiers drift apart.

### 2026-11-03 — Code is named in TAKL's words: full words, one name per concept, `WholeSize::Small` not `I16`, Rust's casing, tests named after their decision (owner: "Yes")

`decision.house.naming`

The owner: "Where's naming?" Design claude proposed Radium's full words and one name per concept, and TAKL's words
over textbook ones, so code, decisions and errors speak one language. The owner: "What about const, and stuff". Design
claude proposed Rust's casing, which `clippy` enforces, a golden test named after its decision
(`tests/tables/no_holes.takl`), and error kinds named for what a kid sees (`OutOfRange`, `NotDesignedYet`). The owner:
"Yes". In `IMPLEMENTATION.md`.

### 2026-11-03 — Comments are traceable: `// trace: decision.tables.no_holes`, each decision named under its heading, a check in Rust (owner: "good")

`decision.house.trace`

The owner: "But should comments be tracable too?" Radium tags code `trace D<n>`; TAKL's decisions have no numbers but
each has a name, its commit's subject up to the colon. Design claude proposed that name under each heading here, so a
tag is found by a plain search; the oracle tagged as the compiler is; and a check that fails on a tag naming no
decision and lists the decisions nothing traces yet, a list of what is left to build. The owner: "good". The check is
written in Rust with the oracle, by the language rule.

### 2026-11-03 — The implementation rules: design claude builds the oracle, compiler claude the compiler, in Rust and Cranelift alone, never reading `oracle/src` (owner: "Yes to all, no core limits")

`decision.house.impl_rules`

Design claude offered to start a session for the oracle. The owner: "`impl` usually does the harder ones, like the real
compiler itself. The oracle is usually done by the design session, because they know it best. Let's define the impl
rules". Design claude drafted nine on CyborgPL's and Radium's: Rust and Cranelift only; compiler claude in its own clone
on the same `main`, never editing the decisions, the oracle or the tests; reading the oracle's tests and never
`oracle/src`, lest their bugs agree; nothing undecided guessed, a `NotDesignedYet` error instead; every golden test
passing on the interpreter and Cranelift at both strengths, matching the oracle byte for byte, before a push; a kid's
program never crashing TAKL, which says "TAKL broke, not you"; `rustfmt`, `clippy` and comments saying why; the commit
style; cores. The owner: "Yes to all, no core limits". They are in `IMPLEMENTATION.md`, and `CLAUDE.md` points to it.

### 2026-11-03 — `add 40 to scores` puts it on the end; `add 40 to scores at position 2` puts it there, the rest moving up (owner: "yes")

`decision.tables.add`

```
add 40 to scores                    # [10, 20, 30, 40]
add 40 to scores at position 2      # [10, 40, 20, 30]
add "sam": 10 to ages               # a named key, on the end
```

The owner: "Where". On the end, where the no-holes error points; a named key too, as `for each` goes in the order
things were put in. `at position` and `at offset` count as they do everywhere. Design claude proposed both; the owner:
"yes".

### 2026-11-03 — A decimal prints as it is held: an exact `2.50` as `2.50`, a fast `~0.1` as `0.1000000000000000055511151231257827021181583404541015625` (owner: "correct")

`decision.show.decimals`

Design claude proposed exact decimals as written and fast ones in the shortest form that reads back the same. The
owner: "print like how it's stored". So a fast decimal prints its whole held value, long, every digit true.

### 2026-11-03 — A `, fast` decimal refuses a number it cannot hold exactly; `~0.1` asks for the nearest, on written numbers only (owner: "correct")

`decision.types.approx`

```
let a: medium_decimal, fast = 0.5     # fine, held exactly
let b: medium_decimal, fast = 0.1     # error: 0.1 cannot be held exactly
let c: medium_decimal, fast = ~0.1    # about 0.1, the nearest one
```

The owner: "Should we refuse `0.1` in Binary, and there's an override, like `~` in front". As an exact decimal refuses
`1 / 3`, nothing is rounded unasked: the error names the nearest, "0.1000000000000000055511151231257827…", and
`~0.1`. `~` is for a number written in the program; the maths of fast decimals still rounds, `a plus b`, since that is
what `, fast` asks for, and `~` on every sum would leave them unusable. Design claude asked whether that is what the
owner meant; the owner: "correct".

### 2026-11-03 — A fraction is written `1 over 3`, and `show` prints it so: `1 over 3` (owner: "yes")

`decision.types.over`

With `/` answered "did you mean", `over` is one word and how a fraction is read aloud; what `show` prints can be typed
back in. Design claude proposed it; the owner: "yes". Not designed: whether `2 over 4` is shown as `1 over 2`.

### 2026-11-03 — `show` prints every number too: `show x` prints `500` (owner)

`decision.show.numbers`

The owner's first program shows a `small_whole`. Asked whether `show` prints whole numbers, the owner: "can also print
all numbers": wholes, decimals of both kinds and fractions. A whole number prints as its digits, a fraction as `1 over 3`, a decimal as it is
held (the entries above).

### 2026-11-03 — `show` prints its pieces one after another, commas between them: `show "Hello, ", name` (owner)

`decision.show.pieces`

```
show "Hello, World!"
show "Hello, ", name
```

Design claude proposed how `show` prints each type. The owner: "There's only this `show` for now". Text prints without
its quotes; nothing is put between the pieces, so the space is the kid's, inside `"Hello, "`. Numbers: the entry
above. Not designed: how a table or `true` prints, and whether `show` ends its line.

### 2026-11-03 — Text is written in double quotes: `show "Hi"` (owner: "yes")

`decision.text.quotes`

`'` already belongs to `can't_change` and `isn't`, so `"` is the one that clashes with nothing; autocorrect's `“ ”`
are read as `"`, as decided. Design claude proposed it; the owner: "yes".

### 2026-11-03 — `remainder of (7 divided by 2)` is 1, in the `of` family (owner: "yes")

`decision.maths.remainder`

Read as maths class says it, "the remainder of 7 divided by 2"; the division is bracketed by the bracket rule. Design
claude proposed it; the owner: "yes".

### 2026-11-03 — `7 divided by 2` on whole numbers is an error unless it divides evenly or is rounded on purpose; a `remainder` word for what is left (owner: "yup")

`decision.maths.whole_divide`

Design claude set out three: 3 with the rest dropped, as C and Java do, 7 ÷ 2 not being 3; an error; 3.5, a whole number
turning into a decimal under the types. Design claude proposed the error, as `1 / 3` is for an exact decimal:
`round (7 divided by 2)`, or the answer kept in a decimal or a fraction; and `remainder`, which kids learn in maths
class, for what is left over. The owner: "yup". How `remainder` is written: the entry above.

### 2026-11-03 — `times` and `divided by` go before `plus` and `minus`, as in maths class; brackets when unsure (owner)

`decision.maths.order`

`a plus b times c` is `a plus (b times c)`. Design claude asked whether to follow maths or refuse the mix as `and` with
`or` is. The owner: "Follow maths, and `()` when unsure": brackets are always allowed, never required here. Earlier
entries' `+` is read as `plus`.

### 2026-11-03 — Maths is words: `plus`, `minus`, `times`, `divided by`; `+ - * /` are answered "did you mean" (owner: "yes")

`decision.maths.words`

```
score = score plus 10
speed times time plus start
total divided by 3
repeat (2 times 3) times:
```

Design claude proposed `+ - * /`, symbols where kids already read them and words for comparisons, where `<` and `>`
are mixed up. The owner: "Adding `*`, and `/` (which normally means fucking `or`), and not adding `<`/`>` is pretty
diabolical". So TAKL is words throughout, as its comparisons are, nothing in it reading as "or". `times` in a
`repeat`'s count needs brackets, `repeat (2 times 3) times:`, by the bracket rule. The owner: "yes". Which goes first: the entry
above.

### 2026-11-03 — Every compile error is shown, those caused by an earlier one hidden, then a last line pointing at the first (owner: "yes")

`decision.errors.all`

```
3 mistakes found. Start with the first one: game.takl:2:5, line 2 column 5.
```

Design claude proposed the first error alone. The owner: "Does that mean you'll have to rerun to find a new error
everytime?" Design claude proposed every separate error, the ones an earlier error causes hidden (a misspelt `lives`
not flagged again on each line using it), as Elm and Rust do, capped near 5. The owner: "Why not just all". With the
knock-on errors hidden, each one left is a real mistake; a kid reads the bottom of a scrolled terminal, so the last line
says how many and where the first is, in both places as every error does. A run-time error stops the program, so there
is only ever one. The owner: "yes".

### 2026-11-03 — `= 5` on a param is its default: `move` alone is `move 5` (owner: "Yes")

`decision.procedures.defaults`

It falls out of the param being written as a `let` is. Design claude proposed it; the owner: "Yes to all, except
errors".

### 2026-11-03 — A param is handed a copy unless it says `share`, and then the call says it too: `reset share scores` (owner: "Yes")

`decision.procedures.share`

```
procedure reset (board: table of small_whole, share):
  ...
end procedure reset

reset share scores
```

Otherwise a procedure hands its change back with `return`, `hp = heal hp`. Both sides write `share`, so nothing is
changed without a word at the call. Design claude proposed it; the owner: "Yes to all, except errors".

### 2026-11-03 — `repeat forever:` ... `end repeat`; `stop`, or `stop if ...`, leaves the innermost loop (owner: "Yes")

`decision.loops.forever`

As Easium's D41, with TAKL's `end`; `stop if` is the line ending in `if`. Design claude proposed it; the owner:
"Yes to all, except errors".

### 2026-11-03 — No holes: setting past a table's end is refused, "xs has 3 items, use `add` to put one on the end" (owner: "Yes")

`decision.tables.no_holes`

Design claude proposed it against Lua's holes, where a table's length stops being true. The owner, of a quick round:
"Yes to all, except errors". Putting an item on the end: `add`, the entry above.

### 2026-11-03 — `and` and `or` short-circuit; `and also` and `or also` always check both sides (owner: "yes", "good")

`decision.conditions.short_circuit`

```
if i is at most length of xs and position i of xs is 0:     # the second half only when i is in range
if ask_number is 0 or also ask_number is 1:                 # always asks twice
```

Asked whether they short-circuit, design claude proposed yes, which a guard like the first line needs and every
mainstream language does, a skipped half not running. The owner: "yes, and there should be a no sc ver". Design claude
proposed `and also` and `or also`, two grammar words each, "also" telling a kid the second half runs whatever happens;
Ada has the reverse, its `and then` the short-circuit one. The owner: "good". Mixing any `and` with any `or` still needs
brackets.

### 2026-11-03 — `not` goes in front of one name or a bracketed test: `if not done:`, `not (lives is 0 and score is 0)` (owner: "yes")

`decision.conditions.not`

With `isn't` there, `not` is mostly for a `true_or_false`. Design claude proposed it under the bracket rule; the owner:
"yes".

### 2026-11-03 — Tests join with `and` and `or`; mixing the two needs brackets (owner: "yes")

`decision.conditions.and_or`

```
if (lives is 0 and score is 0) or time is 0:     # fine
if lives is 0 and score is 0 or time is 0:       # error: add brackets
```

Whether `a and b or c` is `(a and b) or c` or `a and (b or c)` is refused rather than remembered, as
`position i + 1 of xs` is; Easium had left it open. Design claude proposed it; the owner: "yes". `not`: the entry above.

### 2026-11-03 — `if lives is 0:` ... `else if lives is 1:` ... `else:` ... `end if` (owner: "yup")

`decision.conditions.if`

```
if lives is 0:
  show "Game over"
else if lives is 1:
  show "Last life!"
else:
  show "Keep going"
end if
```

In `procedure`'s shape. Easium's `else_if` would be glued grammar, so under TAKL's rule it is `else if`, two grammar
words as `at least` is. Design claude proposed it; the owner: "yup".

### 2026-11-03 — `lives is more than 5`, `less than 5`, `at least 5`, `at most 5` (owner: "yes")

`decision.conditions.compare`

Design claude proposed `above`, `below`, `5 or more` and `5 or less`, worried that `or` would also join tests. The
owner: "Why not `at least`?" Design claude had been inconsistent: `at least` is two grammar words, as `for each` and
`repeat until` are, and the rule refuses only gluing grammar, not grammar of two words. The owner: "Why not `more
than`?" It is maths class's word, `above` and `below` being about places. The owner: "yes".

### 2026-11-03 — `lives is 0`, `lives isn't 0`; `is not` is answered "did you mean `isn't`?" (owner: "yea")

`decision.conditions.isnt`

Easium's `is_not` would be grammar of two pieces, which TAKL's rule refuses. The owner: "Why is there no `_`?" Design
claude: `_` is for names alone, and gluing grammar would bring back the exception `end_of` was. The owner: "Why not
`isn't`?" One word, as kids say it, beside `can't_change`; the apostrophe rule reads `isnt` and a curly `isn’t` as it.
Design claude proposed it as the only form; the owner: "yea".

### 2026-11-03 — A line can end in `if`: `show "hi" if lives is 0` (owner)

`decision.conditions.trailing_if`

The owner: "Also, `show \"hi\" if...` is legal". A line runs only when the test after its `if` holds. Design claude
reads it as any line, `stop if lives is 0` among them. `if` as a block: the entry above.

### 2026-11-03 — `repeat until lives is 0:` ... `end repeat`, checked before each round; no `while` (owner: "yes")

`decision.loops.until`

```
repeat until lives is 0:
  ...
end repeat
```

The owner had design claude ask Easium's design claude: Easium's D39 is `repeat until lives is 0:`, checked before each
round, so the body may run no times, and no `while`, since `until` with the opposite test is the same loop. Design
claude proposed it for TAKL with TAKL's `end`. The owner: "yes". Not designed: `repeat forever:` with `stop` for leaving
early (Easium D41), and the words a test is written with, since Easium's `is_not`, `at_least`, `greater_than` are grammar
of several pieces, which TAKL's rule refuses.

### 2026-11-03 — `= a` copies, `= share a` is the same table; memory is reference counting, copy-on-write, and a cycle collector (owner: "yes")

`decision.memory.share`

```
let b: table of small_whole = a          # a copy, as a number is
let b: table of small_whole = share a    # the same table: change b, a changes too
```

The owner: "What memory model? GC, RC, Ownership, Arena, Manual?" Design claude proposed value semantics on reference
counting and copy-on-write, against the beginner's "I changed `b` and `a` changed too!", with no cycles and so no
collector. The owner: "I mean, GC is very beginner friendly". Design claude: so is a copy, neither asks a kid to free
anything; the difference is whether `b = a` shares, as Python's, JavaScript's and Ruby's do, and the collector beneath is
the implementation's. The owner: "`b = share a`, `b = copy a`, whatever that is. Idk". Design claude proposed a bare
`= a` copying, the safe and common case, and `share` written when it is meant, as `, fast` and `can't_change` are;
no `copy`, as there is no `normal`. Sharing allows cycles, an enemy aiming at a player aiming back, which plain counting
leaks, so a cycle collector runs beneath, as Python's does; a kid never sees it. The owner: "yes".

### 2026-11-03 — Error messages take Radium's layout, made for kids: `Stopped before running:` or `Stopped while running:`, the place, the code and a `^`, then `what:`, `why:`, `when:`, `because:` (owner: "yes")

`decision.errors.layout`

The owner had design claude ask Radium's design claude, whose decisions are Radium's D114, D116 to D119, D122 and D123:
a first line of the kind and the place twice, `file:line:column` for editors (the column in UTF-8 bytes) then
`line N column M` for people (in grapheme clusters); the code line with a `^` run under exactly the part, counted in
grapheme clusters; lined-up `what:` (always), `why:` (always), `when:` (the values, where there are some); `fix:` in
place of `what:` only when there is exactly one fix, code to copy, a list of fixes thrown out as more places to be
wrong; `because:` links, the earlier steps that led here, each with its place, code and pointer, in the order they
happened; one layout at compile time and run time, the kind word alone differing; only what is certain is said.

Design claude proposed copying it with three changes for kids: the kind words are `Stopped before running:` and
`Stopped while running:`; `why:` gives the rule in plain words with no decision number, and bridges to other
languages ("other languages call this an int16"); "did you mean" is Radium's `fix:`, given only when one near match
is certain, several near matches getting the facts and no guess. The owner: "yes". How many errors are shown at once:
the entry above.

### 2026-11-03 — One IR between bytecode and Cranelift, at two strengths: warm code gets few passes and Cranelift's quick mode, very hot code every pass and its speed mode (owner: "yes")

`decision.backend.two_strengths`

```
bytecode → TAKL's IR → Cranelift
```

The owner: "should there be two IR to feed to Cranelift? A less optimized, and a very optimized?" Design claude
proposed one IR worked at two strengths: bytecode is poor to optimize over and Cranelift's IR has lost TAKL's types,
so what knows TAKL, a `+ 1` that cannot overflow, inlining, is done in between; two strengths win back some of what
LLVM gave in very hot code, and two IRs would write each optimization twice, the drift between tiers again. The owner:
"yes". Not designed: what warm and very hot are, and which passes each strength runs.

### 2026-11-03 — The shared form is bytecode: the interpreter runs it, Cranelift compiles from it (owner: "yes")

`decision.backend.bytecode`

The owner: "What's the diff between Bytecode and IR?" Design claude: an IR is any form between source and machine code,
most often shaped for a compiler to reason over; bytecode is the kind shaped for running, a compact list an
interpreter steps through, as Python's, the JVM's and Lua's are. JITs keep both (V8, the JVM, LuaJIT): bytecode run and
counted, the hot parts taken into the compiler's own IR. So TAKL's one shared form is bytecode; the interpreter runs it,
counting what is hot, Cranelift compiles from it with those counts, and every place in compiled code maps back to a
place in the bytecode, which is how a loop is jumped into mid-way. The oracle still walks the parsed source.

### 2026-11-03 — No LLVM: the interpreter, then Cranelift for what gets hot; TAKL is written in Rust (owner: "Drop LLVM")

`decision.backend.no_llvm`

The owner asked whether the interpreter should be C for portability and the tiers Rust and C++. Design claude
recommended Rust throughout: Rust already reaches where kids are, Windows, Mac, Linux, Chromebooks, iPads and the
browser through WebAssembly, and tiers sharing values and jumping in mid-loop across C and Rust define every layout
twice. On iPads and in the browser no JIT is allowed, so there the interpreter is TAKL and must stand alone. The owner:
"We could drop C, but what about custom LLVM passes?" They run in a JIT as well as ahead of time, but only in C++;
design claude recommended TAKL's own passes on its own IR in Rust, before either backend, then a C++ wrapper through
the `cxx` crate if ever needed. The owner: "Or should we drop LLVM?" Design claude leaned yes: LLVM is faster in very hot
loops, often by something like 1.2 to 2 times on number work, at the cost of a dependency of hundreds of megabytes, its
C++ churn and a third compiled engine to match, and Cranelift writes object files too, so a kid's program can still
become an app. The owner: "Drop LLVM". With it goes C++. This narrows the tiers entry below to two tiers; the bytecode
asked about is not decided.

### 2026-11-03 — Three tiers: an interpreter first, Cranelift for what gets hot, LLVM for what gets very hot, compiled in the background on other cores (owner)

`decision.backend.tiers`

LLVM dropped the same day: the entry "No LLVM" above.

The owner: "Tier 0 and oracle interp wouldn't be the same, right?" Design claude: no, the oracle reads the parsed
source and is written to be plainly right, a tier 0 would read the shared IR and be built for speed, and one program
for both would agree with its own bugs. Design claude recommended no tier 0, Cranelift compiling each procedure on its
first call; the owner: "we could do interp first, and compile in background", then "Like, start with interp, and scan for
hot to compile in background, and maybe LLVM for very hot. On other cores, if that has better perf". Design claude
set out the costs: a third engine to match the others to the letter, and a hot loop at the top level reaching compiled
code only by jumping in mid-loop (on-stack replacement), the hardest part of the JIT. Chosen anyway, by the owner.

So the interpreter runs at once, counts what is hot, and hands it to Cranelift on another core, then very hot code to
LLVM; the program jumps in mid-loop when the code is ready. The oracle stays its own program. Every tier passes the one
suite, with a flag forcing each tier, Cranelift and LLVM, from the start. Compiling on other cores is to be measured
against compiling in place. Not designed: what counts as hot and very hot, and which tier is built first. This widens
the backend entry below.

### 2026-11-03 — Testing: golden tests, a reference interpreter as oracle, a forced second tier, and fuzzing, all of them (owner: "All of them.")

`decision.tests.plan`

The owner: "How should we test? Like the impl". Design claude proposed four, and asked about the reference interpreter
first, the most work up front. The owner: "All of them."

1. Golden tests: a small `.takl` program beside what it must print, a kid-facing error message counted as output to
   the letter, since the messages are what TAKL is. Every example in this log is one, so the log and the tests cannot
   drift apart.
2. A reference interpreter, slow and plain, walking the program as CyborgPL's oracle does; every test runs through it
   and the JIT, their output matching byte for byte.
3. A flag sending everything to the second tier at once, so the LLVM tier is tested on every program and not only hot
   loops, as the backend entry requires.
4. Fuzzing: random number programs through the interpreter and both tiers, which must agree; where `127 + 1` lives.

### 2026-11-03 — A block ends with `end` and what it closes: `end procedure move`, `end repeat`, `end for`, `end table level`; `end_of` is gone (owner: "yes")

`decision.blocks.end`

```
procedure move (speed: small_whole):
  show speed
end procedure move
```

Under the rule that a grammar word is one word, `end_of` was the one exception, and `end of` would be read as the `of`
family. Design claude proposed dropping the `of`: `end` is one grammar word, never mistaken for `sum of`, and still names
what it closes, at the cost of a little English. The owner: "yes". It replaces `end_of` in every entry below.

### 2026-11-03 — A name is glued with `_`; a grammar word stands alone: `small_whole`, `middle_of`, but `sum of scores` (owner: "yes")

`decision.names.underscore`

Names are what is declared, used or looked up: types, modifiers, variables, procedures, `small_whole`, `true_or_false`,
`can't_change`, `never_negative`, `first_name`, `middle_of`. Grammar words are TAKL's own fixed words, each one word:
`let`, `show`, `of`, `in`, `for`, `each`, `repeat`, `times`, `position`, `offset`, `procedure`, `returns`, `return`,
`end`. A kid asks: did someone name this, or is it part of TAKL's sentence? Only TAKL's built-ins take grammar, so
`sum of scores` keeps a bare `of` and a kid's `middle_of` glues it.

The owner had design claude ask Easium's design claude, whose rule (Easium D5, D25, D34) is that one thing never holds a
space and words join when neither can be swapped for another; under it `small_whole` was unclear, `small` swapping with
`tiny`. The owner: "Then, we need a clearer rule". Design claude proposed this one, with `end_of` its one exception; the
owner: "Then, why is it not `end of`?", and the exception went (the entry above). The owner: "yes".

### 2026-11-03 — A kid's own `of` is only a name, `procedure middle_of (xs: table of small_whole)`, called `middle_of scores`; a bare `of` is the built-in family's alone (owner)

`decision.procedures.own_of`

Design claude proposed `procedure middle of (...)`. The owner: "Is that a space in a fn name?" Design claude offered any
one-param procedure called with `of`, or `of` before the param, `(of xs: ...)`. The owner: "Or just no `of` at all", then
"They could just name it `middle_of`". So a kid's own `of` is no new form, only a name with `_of` in it; the built-in
family keeps its bare `of`, `sum of scores`. `sum_of scores` and `middle of scores` are typos, answered "did you mean".
This replaces "kids can write their own `of`" in the `of` family entry below.

### 2026-11-03 — `end_of` stays one word, so it is never read as the `of` family (owner: "keep")

`decision.blocks.end_of`

Replaced the same day by `end`, the entry above.

Shown a kid's own `procedure middle of (...)`, the owner: "Wait, if that one uses a bare `of`, why not the `end`?" Design
claude: `end of procedure middle` would read as the `of` family, `end of scores` looking like the last item, and a kid
writing `procedure end of (...)` would leave the compiler unable to tell the two apart. The owner: "keep".

### 2026-11-03 — The `of` family, `length of`, `sum of`, `biggest of`, `smallest of`, `average of`, beside one in-order `for each` loop; kids can write their own `of` (owner: "yea")

`decision.tables.of_family`

```
show sum of scores

for each score in scores:
  show score
end_of for

for each name and age in ages:
  show name
end_of for
```

Design claude proposed `for each`, one loop in place of Lua's `pairs` and `ipairs`, always in order: 1, 2, 3 for a
numbered table, the order things were put in for a named one. The owner: "For that, you could just say `sum of`,
`leghth of`, wait... There could be a whole family". Design claude proposed the five, under the bracket rule
(`sum of (position 1 of grid)`), and `for each` kept for what no word does, showing each name or moving each enemy;
asked whether the family looks right and whether kids can write their own, the owner: "yea". Not designed: how a kid's
own `of` is written.

### 2026-11-03 — `repeat 50 times:` ... `end_of repeat`; a count from a variable is `repeat lives times:` (owner: "good")

`decision.loops.repeat`

```
repeat 50 times:
  show "Hi"
end_of repeat
```

Design claude proposed it in `procedure`'s shape, the first of the general loops, a count from a variable reading as
plainly as a number. The owner: "good". Not designed: a loop over a table, a loop while something holds.

### 2026-11-03 — No `show "Hi" 50x` shortcut (owner: "drop shortcut")

`decision.loops.no_shortcut`

The owner proposed `show "Hi" 50x` printing it 50 times, a shortcut before general loops. Design claude raised a count
from a variable (`show "Hi" lives x`, or `(lives)x` by the bracket rule) and `x` clashing with multiplying, CyborgPL's
`'price' x 'tax'`. The owner: "Nvm", then, asked whether all of it or the count only: "drop shortcut".

### 2026-11-03 — The type of true and false is `true_or_false`: `let done: true_or_false = false` (owner)

`decision.types.true_or_false`

Design claude proposed `yes_or_no`, keeping `true` and `false` as the values since they carry over everywhere, with an
error still able to say "other languages call this a boolean". The owner wrote `let done: true_or_false = false`: the
name says the very values it holds.

### 2026-11-03 — An exact decimal never rounds on its own: `1 / 3` is an error, `round (1 / 3)` rounds on purpose, as in CyborgPL (owner: "copy")

`decision.decimals.rounding`

```
let slice: small_decimal = round (1 / 3)    # fine
let slice: small_decimal = 1 / 3            # error: this goes on forever, round it or use a fraction
```

Design claude first proposed rounding to the type's digits on its own, so a kid sharing a pizza three ways is not
stopped. The owner: "I mean, have we done that before?" Not in TAKL; in CyborgPL, yes: a decimal with `max_decimals`
needs a rounding said on purpose, `round.near[..., decimals 2]` and four other modes (CyborgPL, 2026-10-17). Asked whether
to copy that or round on its own, the owner: "copy". A result that fits the type's digits, `1 / 4` as 0.25, is not
rounded and needs nothing. Not designed: the rounding modes, rounding to a number of places, and whether the error comes
at compile time, run time or both.

### 2026-11-03 — A bare `small_decimal` is exact; `, fast` is the only kind ever written, `normal` is not a word (owner: "yup")

`decision.types.exact_bare`

Design claude proposed that a decimal with nothing after it be exact, as maths class is, so a kid never types `normal`;
the owner: "yup".

### 2026-11-03 — Decimals and fractions take the whole numbers' sizes: `medium_decimal, fast` is a float64, `small_decimal` exact, `small_fraction` two small_wholes; no 8-bit float (owner: "yes")

`decision.types.decimals`

| size | `, fast` (a float) | exact | `_fraction`, top and bottom each a |
|---|---|---|---|
| tiny | float16 | 7 digits | tiny_whole |
| small | float32 | 16 digits | small_whole |
| medium | float64 | 34 digits | medium_whole |
| big | float128 | about 70 digits | big_whole |

Design claude set out real floats (`0.1 + 0.2` is `0.30000000000000004`) against exact decimals (`0.3`, slower). The
owner proposed `fast_decimal` and `normal_decimal`, then "we would add sizes too, and also rationales", and then, of
design claude's `big_normal_decimal`: "Where are the other names, and I mean we could do `big_decimal`, and then `fast` or
`normal`". So a decimal is named by size as a whole number is, and what kind it is goes after the comma; a rational is a
`fraction`, a word kids know from school, exact for `1/3`. Design claude kept float128 for the ladder's sake, though most
CPUs run it in software and it is not fast, the help to say so. Asked about binary8: it is not in IEEE 754; 8-bit floats
are OCP FP8's two and IEEE P3109's several, about 2 digits, run by no CPU. Design claude recommended leaving it out and
`tiny_decimal, fast` a float16. The owner: "yes".

### 2026-11-03 — TAKL takes inspiration from Ruby, and that is all (owner)

`decision.house.ruby`

Design claude asked what TAKL takes from Ruby beyond `show` without brackets, `#` comments and no semicolons. The owner:
"TAKL takes inpirsation from Ruby, that's all." Nothing is to be carried over from Ruby for being Ruby's.

### 2026-11-03 — The text type is `text`, never `string` (owner: "yes")

`decision.text.name`

Design claude proposed `text` over `string`: "string" means nothing to a kid, `text` is what it is, and an error can
still say "other languages call this a string". The owner: "yes".

### 2026-11-03 — Text is always valid UTF-8, counted in grapheme clusters, letters as a kid sees them: `"👍🏽"` is 1 long (owner: "yes")

`decision.text.letters`

Design claude proposed that a position into text and its length count letters as seen, never bytes or code points, which
are reached only by an explicit step. The owner: "So, we would normally do grapheme clusters?" Design claude: yes, at two
costs, `position 1000 of name` walking the text from its start, and Unicode's segmentation tables shipped with the
compiler and updated with each Unicode version (CyborgPL generates the same tables from unicode.org). Asked whether the
speed is fine, the owner: "yes". The words for code points and for turning text into bytes are not designed.

### 2026-11-03 — A table's item type with modifiers is bracketed: `table of (tiny_whole, never_negative)`; a modifier outside is the variable's (owner: "yes")

`decision.tables.item_modifiers`

```
let data: table of (tiny_whole, never_negative) = [72, 105]
let data: table of (tiny_whole, never_negative), can't_change = [72, 105]
```

The owner asked whether TAKL needs to tell UTF-8 apart from bytes. Design claude proposed bytes as no type of their own,
a table of `tiny_whole, never_negative`, met when a kid opens a file or an image, and raised that in
`table of tiny_whole, never_negative` the modifier could be the items' or the table's. The owner: "How do we make that
less confusing?" Design claude proposed the index rule again, brackets for what goes together, inside them what each item
is and outside what the variable is; `table of tiny_whole, never_negative` bare is an error asking
"did you mean `table of (tiny_whole, never_negative)`?". The owner: "yes".

### 2026-11-03 — Tables, as Lua's: `table of small_whole` keyed 1, 2, 3, `table of text to small_whole` by name; `[10, 20, 30]` on one line, a block past it (owner: "yes")

`decision.tables.shape`

```
let scores: table of small_whole = [10, 20, 30]
sum [1, 2, 3]

let level: table of text to small_whole:
  "width": 40
  "height": 20
end_of table level

let grid: table of table of tiny_whole:
  [1, 2, 3]
  [4, 5, 6]
end_of table grid
```

The owner proposed "Lua-style tables, but improved", then asked why `[]` when "you can't really misread `10, 20, 30` as a
kid". Design claude answered that bare commas clash with a call's arguments (`sum 10, 20, 30`), tables in tables and
the empty table. The owner proposed blocks, "like how a procedure" is. Design claude proposed both forms: brackets that
never go past the end of their line (past it is an error saying to make it a block) and a block of one item a line with
no commas, the two making the same table. The owner: "yes".

### 2026-11-03 — `return a + b` gives a procedure's value back; the last line is never returned on its own (owner)

`decision.procedures.return`

Design claude proposed a word you can see over Ruby's returning the last line, offering `give_back` or `return`. The
owner: "Just `return` with no `s`? I mean, above already says `returns`". It pairs with `returns` in the first line and is
the word other languages use.

### 2026-11-03 — `procedure add (a: small_whole | b: small_whole) returns small_whole:` ... `end_of procedure add`; called as `add 1, 2` (owner: "yes")

`decision.procedures.shape`

```
procedure move (speed: small_whole, never_negative | height: tiny_whole):
  ...
end_of procedure move
```

A param is written as a `let` is, without the word, modifiers after commas; `|` stands between params, so a comma only
ever joins a param's modifiers. A call has no brackets, as `show` has none (the owner: "print is `show "Hello, World!"`"). Its arguments are
split by commas, `add 1, 2`, since a call has no modifiers to confuse them with and `add 1 | 2` reads as "1 or 2";
design claude first wrote that in unasked, then put it to the owner, who chose commas: "yup".
The owner proposed `procedure`, the brackets, `end_of procedure move`, `|` and `returns`; design claude recommended
`returns` over `->`, since TAKL says things in words and an error can still mention `->`. No `returns` means nothing comes
back. The `:` replaced the owner's first `has_body`; design claude called it redundant beside `end_of`, and the owner:
"`:` is there for... Making reading it with wrong grammar less weird".

### 2026-11-03 — A bare index is one number or one name; anything more is bracketed, `position (i + 1) of xs` (owner: "yes")

`decision.lists.index_bare`

Design claude first proposed reading everything between `position` and `of` as the index. The owner, of
`position i + 1 of xs`: "What the fuck in the stroking shit did I just fucking witnessed". Design claude then proposed
refusing it: `position 1 of xs` and `position i of xs` stand bare, anything more needs brackets, and
`position i + 1 of xs` is a compile error saying "did you mean `position (i + 1) of xs`?". The same holds for `offset`.
The owner: "yes".

### 2026-11-03 — A number past its type's range is a compile error when it can be known, a stopping error at run time, unless an `otherwise` beside it handles it (owner)

`decision.numbers.range`

Design claude proposed catching it at compile time when the numbers are known and stopping the program at the line where
it happens otherwise, with the same message both times. The owner: "spot at compile time, err. runtime default to
error, unless there's a `otherwise` sitting beside, which is err handling". So `otherwise` is TAKL's error handling.
Not designed: how `otherwise` is written, and whether it handles more than numbers out of range.

### 2026-11-03 — `position 1 of xs` is the first item, counting from 1; `offset 0 of xs` is the same item, counting from 0 (owner)

`decision.lists.index`

The owner proposed `total = x + position 1 of xs`, and `offset` beside `position`. Design claude read it as two words, each
with one start: `position` counts from 1 as a person does, `offset` from 0 as other languages do, so a kid starts with
`position` and `offset` shows why the rest of the world starts at 0. The owner: "read right". Each word only ever means
its own start. How an index longer than one number or name is written: the entry above.

### 2026-11-03 — `, never_negative` after a whole number's type makes it unsigned: `small_whole, never_negative` is a uint16, 0 to 65535 (owner)

`decision.types.never_negative`

The owner proposed `let score: small_whole, never_negative`. Asked whether it doubles the top end or only refuses
negatives, the owner: "It's `uint`, without saying `unsigned_small_whole`, and I think stacking it to `small_whole` is too
long". So it is a true unsigned number, written as a word after the comma as `can't_change` is, and never as a longer type
name.

### 2026-11-03 — A `let` can change unless it says `can't_change`; `score = score + 10` changes it (owner: "yes")

`decision.lets.change`

Design claude proposed that a `let` without `can't_change` can change, written `score = score + 10`, since the owner's
first program implies it and `can't_change` is then a choice a kid makes. The owner: "yes, because not many kids would
fucking go 'Oh this is bad, it's not accurate to maths'".

### 2026-11-03 — The whole numbers are tiny_whole, small_whole, medium_whole and big_whole: int8, int16, int32, int64 (owner)

`decision.types.whole`

Design claude proposed `tiny_whole`, `small_whole`, a bare `whole` for int32 as the one kids reach for, and `big_whole`.
The owner: "why not fucking `medium_whole`. I mean, they'll have to learn about those eventually. We just don't fuvking say
`int32`, or even more diabolical `i32`". So there is no bare `whole`: every whole number says its size, in words and never
in bits. Unsigned whole numbers are not designed; design claude suggested waiting until bytes or colours need them.

### 2026-11-03 — Artificial-IntelligenceAI/TAKL is public, with its full history (owner: "Public")

`decision.house.public`

The repository was made private on the owner's account, not Soclets, when it was created, design claude choosing private
since the owner had not said. The owner: "Public".

### 2026-11-03 — The code is Apache 2.0 with the LLVM exception; the docs are CC BY 4.0 (owner)

`decision.house.license`

The owner: "Apache 2.0 with LLVM, and CC BY 4.0 for docs". The code, the compiler and anything a program is built with, is
under the Apache License 2.0 with the LLVM exception (LICENSE); the docs, this log among them, under Creative Commons
Attribution 4.0 International (LICENSE-CC-BY-4.0). Both texts as CyborgPL has them, unchanged.

### 2026-11-03 — Cranelift now; one IR that both backends read; LLVM later as a second tier (owner: "Yeah, seems good")

`decision.backend.cranelift`

LLVM dropped the same day: the entry "No LLVM" above.

The owner proposed tiering Cranelift and LLVM ("fucking insane for a Scratch-ass Ruby fucking language"). Design claude
recommended building Cranelift alone first, with one IR shaped for two backends so LLVM is added later rather than
rewritten in; kids' game loops get hot, so a second tier is not pointless. Both tiers pass one test suite with no
exception: a program that behaves differently once it warms up is a bug no kid can find. The owner: "Yeah, seems good".

### 2026-11-03 — A real typo stops the program, with a "did you mean" (owner: "stop, obviously")

`decision.spelling.typos`

Narrowed the same day by the error-message entry above: "did you mean" only when one near match is certain.

Design claude proposed that `cant_chang` be answered with "did you mean `can't_change`?" and never guessed and run, since
a guess teaches that sloppy spelling works. Asked whether that stops the program or only warns, the owner: "stop,
obviously".

### 2026-11-03 — `can't_change` and `cant_change` are both read; what autocorrect changes is read as typed (owner: "accept both seems like a good idea", "some autocorrect protection")

`decision.spelling.cant_change`

Design claude raised that kids will type `cant_change`, and that text pasted from a document or a chat carries a curly
`’`. The owner: "accept both seems like a good idea, and maybe we should have some autocorrect protection". Design claude
proposed reading autocorrect's changes as what was meant, with no message: `’` as `'`, `“ ”` as `"`, `—` as `-`, because
that was autocorrect's doing and not the kid's; a real typo is the next entry. The owner did not object.

### 2026-11-03 — `let` names a type before its value; `small_whole` is an int16; `show` prints; `#` starts a comment (owner)

`decision.syntax.let`

The owner's first program:

```
let x: small_whole, can't_change = 500 #it's an int16
show x
```

The owner: it teaches types before params, "so, they won't be scared to write params". Not designed: the other types,
what a `let` without `can't_change` may do.

## Open

- How `otherwise` is written and what it handles; the owner: "`otherwise` won't be design rn".
