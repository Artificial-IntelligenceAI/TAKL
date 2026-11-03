# TAKL — Design decisions

Running log. Each entry: date, the decision, who proposed it, and who chose it.
Nothing here is final until the owner says so; open questions live at the bottom.

## Decided

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
- The other type names, beside `small_whole`.
- Whether a `let` without `can't_change` can change.
- What a number too big for its type does.
- What TAKL takes from Ruby beyond `show` without brackets, `#` comments and no semicolons.
