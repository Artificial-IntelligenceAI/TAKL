# TAKL

Tankun's Ambioustious Kids Language (the misspelling is intended). It is a JIT language for kids, inspired by Ruby,
that teaches types before params.

- `DECISIONS.md`: every design decision, with the owner's words, newest first; open questions at the bottom. Nothing
  is decided until the owner says so.
- `IMPLEMENTATION.md`: the rules for the oracle and the compiler, and which session does what. Read it in full before
  contributing code.

## Git

- Commit subjects are `kind.folder.narrowing: sentence (owner: "their words")`. A decision is one commit adding one
  entry to `DECISIONS.md`.
- Each commit ends with its session's trailer, `Co-Authored-By: design claude <noreply@anthropic.com>` or
  `compiler claude`.
- The remote is `https://github.com/Artificial-IntelligenceAI/TAKL` (public). Push over HTTPS with gh as the
  credential helper; SSH is refused for this account.

## Licences

Code is Apache 2.0 with the LLVM exception (`LICENSE`); docs are CC BY 4.0 (`LICENSE-CC-BY-4.0`).
