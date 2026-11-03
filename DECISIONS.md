# TAKL — Design decisions

Running log. Each entry: date, the decision, who proposed it, and who chose it.
Nothing here is final until the owner says so; open questions live at the bottom.

## Decided

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
