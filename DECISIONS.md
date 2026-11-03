# TAKL — Design decisions

Running log. Each entry: date, the decision, who proposed it, and who chose it.
Nothing here is final until the owner says so; open questions live at the bottom.

## Decided

### 2026-11-03 — A `let` can change unless it says `can't_change`; `score = score + 10` changes it (owner: "yes")

Design claude proposed that a `let` without `can't_change` can change, written `score = score + 10`, since the owner's
first program implies it and `can't_change` is then a choice a kid makes. The owner: "yes, because not many kids would
fucking go 'Oh this is bad, it's not accurate to maths'".

### 2026-11-03 — The whole numbers are tiny_whole, small_whole, medium_whole and big_whole: int8, int16, int32, int64 (owner)

Design claude proposed `tiny_whole`, `small_whole`, a bare `whole` for int32 as the one kids reach for, and `big_whole`.
The owner: "why not fucking `medium_whole`. I mean, they'll have to learn about those eventually. We just don't fuvking say
`int32`, or even more diabolical `i32`". So there is no bare `whole`: every whole number says its size, in words and never
in bits. Unsigned whole numbers are not designed; design claude suggested waiting until bytes or colours need them.

### 2026-11-03 — Artificial-IntelligenceAI/TAKL is public, with its full history (owner: "Public")

The repository was made private on the owner's account, not Soclets, when it was created, design claude choosing private
since the owner had not said. The owner: "Public".

### 2026-11-03 — The code is Apache 2.0 with the LLVM exception; the docs are CC BY 4.0 (owner)

The owner: "Apache 2.0 with LLVM, and CC BY 4.0 for docs". The code, the compiler and anything a program is built with, is
under the Apache License 2.0 with the LLVM exception (LICENSE); the docs, this log among them, under Creative Commons
Attribution 4.0 International (LICENSE-CC-BY-4.0). Both texts as CyborgPL has them, unchanged.

### 2026-11-03 — Cranelift now; one IR that both backends read; LLVM later as a second tier (owner: "Yeah, seems good")

The owner proposed tiering Cranelift and LLVM ("fucking insane for a Scratch-ass Ruby fucking language"). Design claude
recommended building Cranelift alone first, with one IR shaped for two backends so LLVM is added later rather than
rewritten in; kids' game loops get hot, so a second tier is not pointless. Both tiers pass one test suite with no
exception: a program that behaves differently once it warms up is a bug no kid can find. The owner: "Yeah, seems good".

### 2026-11-03 — A real typo stops the program, with a "did you mean" (owner: "stop, obviously")

Design claude proposed that `cant_chang` be answered with "did you mean `can't_change`?" and never guessed and run, since
a guess teaches that sloppy spelling works. Asked whether that stops the program or only warns, the owner: "stop,
obviously".

### 2026-11-03 — `can't_change` and `cant_change` are both read; what autocorrect changes is read as typed (owner: "accept both seems like a good idea", "some autocorrect protection")

Design claude raised that kids will type `cant_change`, and that text pasted from a document or a chat carries a curly
`’`. The owner: "accept both seems like a good idea, and maybe we should have some autocorrect protection". Design claude
proposed reading autocorrect's changes as what was meant, with no message: `’` as `'`, `“ ”` as `"`, `—` as `-`, because
that was autocorrect's doing and not the kid's; a real typo is the next entry. The owner did not object.

### 2026-11-03 — `let` names a type before its value; `small_whole` is an int16; `show` prints; `#` starts a comment (owner)

The owner's first program:

```
let x: small_whole, can't_change = 500 #it's an int16
show x
```

The owner: it teaches types before params, "so, they won't be scared to write params". Not designed: the other types,
what a `let` without `can't_change` may do.

## Open

- Whether a parameter is written exactly as a `let` is (`n: small_whole`); design claude proposed it, the owner has not said.
- The types that are not whole numbers, and whether unsigned whole numbers exist.
- What a number too big for its type does.
- What TAKL takes from Ruby beyond `show` without brackets, `#` comments and no semicolons.
