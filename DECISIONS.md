# TAKL — Design decisions

Running log. Each entry: date, the decision, who proposed it, and who chose it.
Nothing here is final until the owner says so; open questions live at the bottom.

## Decided

### 2026-11-03 — Text is always valid UTF-8, counted in grapheme clusters, letters as a kid sees them: `"👍🏽"` is 1 long (owner: "yes")

Design claude proposed that a position into text and its length count letters as seen, never bytes or code points, which
are reached only by an explicit step. The owner: "So, we would normally do grapheme clusters?" Design claude: yes, at two
costs, `position 1000 of name` walking the text from its start, and Unicode's segmentation tables shipped with the
compiler and updated with each Unicode version (CyborgPL generates the same tables from unicode.org). Asked whether the
speed is fine, the owner: "yes". The words for code points and for turning text into bytes are not designed.

### 2026-11-03 — A table's item type with modifiers is bracketed: `table of (tiny_whole, never_negative)`; a modifier outside is the variable's (owner: "yes")

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

Design claude proposed a word you can see over Ruby's returning the last line, offering `give_back` or `return`. The
owner: "Just `return` with no `s`? I mean, above already says `returns`". It pairs with `returns` in the first line and is
the word other languages use.

### 2026-11-03 — `procedure add (a: small_whole | b: small_whole) returns small_whole:` ... `end_of procedure add`; called as `add 1, 2` (owner: "yes")

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

Design claude first proposed reading everything between `position` and `of` as the index. The owner, of
`position i + 1 of xs`: "What the fuck in the stroking shit did I just fucking witnessed". Design claude then proposed
refusing it: `position 1 of xs` and `position i of xs` stand bare, anything more needs brackets, and
`position i + 1 of xs` is a compile error saying "did you mean `position (i + 1) of xs`?". The same holds for `offset`.
The owner: "yes".

### 2026-11-03 — A number past its type's range is a compile error when it can be known, a stopping error at run time, unless an `otherwise` beside it handles it (owner)

Design claude proposed catching it at compile time when the numbers are known and stopping the program at the line where
it happens otherwise, with the same message both times. The owner: "spot at compile time, err. runtime default to
error, unless there's a `otherwise` sitting beside, which is err handling". So `otherwise` is TAKL's error handling.
Not designed: how `otherwise` is written, and whether it handles more than numbers out of range.

### 2026-11-03 — `position 1 of xs` is the first item, counting from 1; `offset 0 of xs` is the same item, counting from 0 (owner)

The owner proposed `total = x + position 1 of xs`, and `offset` beside `position`. Design claude read it as two words, each
with one start: `position` counts from 1 as a person does, `offset` from 0 as other languages do, so a kid starts with
`position` and `offset` shows why the rest of the world starts at 0. The owner: "read right". Each word only ever means
its own start. How an index longer than one number or name is written: the entry above.

### 2026-11-03 — `, never_negative` after a whole number's type makes it unsigned: `small_whole, never_negative` is a uint16, 0 to 65535 (owner)

The owner proposed `let score: small_whole, never_negative`. Asked whether it doubles the top end or only refuses
negatives, the owner: "It's `uint`, without saying `unsigned_small_whole`, and I think stacking it to `small_whole` is too
long". So it is a true unsigned number, written as a word after the comma as `can't_change` is, and never as a longer type
name.

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

- Lua's warts fixed, as design claude proposed: setting past a table's end refused (no holes), one loop in order instead of `pairs` and `ipairs`.
- The name of the text type; `text` is design claude's, used in examples, never chosen.
- How `otherwise` is written and what it handles; the owner: "`otherwise` won't be design rn".
- Whether `= value` on a param is its default, so `move` alone means `move 5`; design claude proposed it.
- The types that are not whole numbers.
- What TAKL takes from Ruby beyond `show` without brackets, `#` comments and no semicolons.
