//! Every word TAKL says to a kid when something is wrong, kept in one place for the owner to review.
//!
//! The layout is decided (decision.errors.layout, decision.errors.all); the sentences here follow its examples and
//! are design claude's until the owner reviews them.

pub use crate::diagnostic::Stage;
use crate::diagnostic::{Diagnostic, Headline};
use crate::numbers::{Size, Trouble};
use crate::source::Span;
use num_bigint::BigInt;

/// What `show` puts after its pieces.
/// trace: decision.show.line_ending
pub const SHOW_ENDS_WITH: &str = "\n";

fn what(span: Span, what: impl Into<String>, why: impl Into<String>) -> Diagnostic {
    Diagnostic::before(span, Headline::What(what.into()), why)
}

fn fix(span: Span, fix: impl Into<String>, why: impl Into<String>) -> Diagnostic {
    Diagnostic::before(span, Headline::Fix(fix.into()), why)
}

fn at(stage: Stage, span: Span, what: impl Into<String>, why: impl Into<String>) -> Diagnostic {
    match stage {
        Stage::BeforeRunning => Diagnostic::before(span, Headline::What(what.into()), why),
        Stage::WhileRunning => Diagnostic::while_running(span, Headline::What(what.into()), why),
    }
}

/// Something TAKL has not decided yet: it stops rather than guess.
fn not_designed(stage: Stage, span: Span, what: impl Into<String>) -> Diagnostic {
    at(
        stage,
        span,
        what,
        "TAKL hasn't decided how this works yet, so it stops instead of guessing.\nIt's on TAKL's list of open questions.",
    )
}

/// The one name close enough to be the one meant; none when there are several or none.
/// trace: decision.spelling.typos (a typo stops, with "did you mean" only when one near match is certain)
pub fn near(name: &str, candidates: &[String]) -> Option<String> {
    let limit = if name.chars().count() >= 5 { 2 } else { 1 };
    let close: Vec<&String> = candidates.iter().filter(|c| c.as_str() != name && distance(name, c) <= limit).collect();
    match close.as_slice() {
        [one] => Some((*one).clone()),
        _ => None,
    }
}

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut previous = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let current = row[j];
            row[j] = (row[j] + 1).min(row[j - 1] + 1).min(previous + usize::from(a[i - 1] != b[j - 1]));
            previous = current;
        }
    }
    row[b.len()]
}

/// The name other languages give a whole number type.
fn other_name(size: Size, never_negative: bool) -> String {
    format!("{}int{}", if never_negative { "u" } else { "" }, size.bits())
}

// ---- the last line, and TAKL itself ----------------------------------------------------------------------------

/// trace: decision.errors.all
pub fn mistakes_found(count: usize, first: &str) -> String {
    if count == 1 {
        format!("1 mistake found: {}.\n", first)
    } else {
        format!("{} mistakes found. Start with the first one: {}.\n", count, first)
    }
}

/// trace: decision.house.impl_rules (rule 6: a kid's program never crashes TAKL)
pub fn takl_broke() -> String {
    "TAKL broke, not you.\nThis is a mistake in TAKL itself. Please report it, with your program, at\nhttps://github.com/Artificial-IntelligenceAI/TAKL/issues\n".to_string()
}

// ---- writing ---------------------------------------------------------------------------------------------------

pub fn lone_at(span: Span) -> Diagnostic {
    what(span, "`@` needs a name right after it, like `@score`", "`@` marks a variable, and a variable needs a name.")
}

pub fn text_not_closed(span: Span) -> Diagnostic {
    what(span, "this text has no closing `\"`", "Text starts with `\"` and ends with `\"` on the same line.")
}

pub fn backslash_in_text(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "what `\\` does inside text")
}

pub fn mark(span: Span, mark: char) -> Diagnostic {
    let (fix_text, why) = match mark {
        '+' => ("plus", "TAKL writes maths in words."),
        '-' => ("minus", "TAKL writes maths in words."),
        '*' | '\u{00D7}' => ("times", "TAKL writes maths in words."),
        '/' | '\u{00F7}' => ("divided by", "TAKL writes maths in words, and `/` usually means \"or\"."),
        '<' => ("is less than", "TAKL writes comparisons in words."),
        '>' => ("is more than", "TAKL writes comparisons in words."),
        ';' => return semicolon(span),
        _ => {
            return what(
                span,
                format!("TAKL doesn't use `{}`", mark),
                "Only letters, numbers, `@`, `\"`, `:`, `,`, `|`, `=`, `~` and brackets mean something in TAKL.",
            );
        }
    };
    fix(span, fix_text, why)
}

pub fn semicolon(span: Span) -> Diagnostic {
    // trace: decision.syntax.no_semicolons
    what(span, "TAKL lines don't end with `;`", "A line ends where the line ends. Take the `;` out.")
}

pub fn unexpected_here(span: Span, text: &str) -> Diagnostic {
    what(span, format!("`{}` doesn't belong here", text), "TAKL couldn't read the line from this point on.")
}

pub fn expected_value(span: Span) -> Diagnostic {
    what(span, "a value is missing here", "Something like a number, text or a `@variable` goes here.")
}

pub fn expected_word(span: Span, word: &str) -> Diagnostic {
    what(span, format!("`{}` is missing here", word), "TAKL couldn't read the line without it.")
}

pub fn expected_variable(span: Span) -> Diagnostic {
    what(span, "a variable goes here, like `@score`", "Variables start with `@`.")
}

pub fn line_does_nothing(span: Span) -> Diagnostic {
    what(
        span,
        "this line doesn't do anything",
        "A line starts with what it does, like `let`, `show`, `@score =`, or a procedure's name.",
    )
}

pub fn cannot_start_line(span: Span, word: &str) -> Diagnostic {
    what(
        span,
        format!("a line can't start with `{}`", word),
        "A line starts with what it does, like `let`, `show`, `if` or `repeat`.",
    )
}

pub fn outside_procedure(span: Span) -> Diagnostic {
    what(
        span,
        "this line is outside every procedure, so it would never run",
        "A program runs from `procedure start_running`, and every line lives inside a procedure.",
    )
}

pub fn bracket_not_closed(span: Span) -> Diagnostic {
    what(span, "this `(` is never closed", "Every `(` needs a `)` on the same line.")
}

pub fn square_not_closed(span: Span) -> Diagnostic {
    // trace: decision.tables.shape
    what(
        span,
        "this `[` is never closed on its line",
        "A table in `[ ]` stays on one line. For a longer one, make it a block:\nlet @level: table of small_whole:\n  10\n  20\nend table @level",
    )
}

pub fn mixed_keys(span: Span) -> Diagnostic {
    what(
        span,
        "this table mixes items with `key: value` items",
        "A table is either numbered or has names for its keys, not both.",
    )
}

pub fn empty_text_is_empty(span: Span) -> Diagnostic {
    // trace: decision.values.empty
    fix(span, "empty", "Empty text is written `empty`, so it looks like you meant it.")
}

pub fn empty_table_is_empty(span: Span) -> Diagnostic {
    fix(span, "empty", "An empty table is written `empty`, so it looks like you meant it.")
}

pub fn empty_needs_type(span: Span) -> Diagnostic {
    what(
        span,
        "`empty` doesn't know what it's empty of here",
        "`empty` works where the type is known, like `let @name: text = empty`.",
    )
}

pub fn nothing_is_not_a_value(span: Span) -> Diagnostic {
    what(
        span,
        "`nothing` isn't a TAKL word",
        "A procedure that gives nothing back says `just finishes` and ends with `finish`.",
    )
}

pub fn tilde_needs_number(span: Span) -> Diagnostic {
    what(
        span,
        "`~` needs a number right after it, like `~0.1`",
        "`~0.1` means \"about 0.1\", for a `fast_but_inexact` decimal.",
    )
}

pub fn over_needs_whole_numbers(span: Span) -> Diagnostic {
    what(
        span,
        "a fraction is two whole numbers with `over` between them, like `1 over 3`",
        "The top and the bottom of a fraction are whole numbers.",
    )
}

pub fn negative_numbers_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "how a number below zero is written")
}

pub fn is_not_is_isnt(span: Span) -> Diagnostic {
    // trace: decision.conditions.isnt
    fix(span, "isn't", "TAKL writes \"is not\" as one word, `isn't`.")
}

pub fn mix_and_or(span: Span) -> Diagnostic {
    // trace: decision.conditions.and_or
    what(
        span,
        "this mixes `and` with `or` without brackets",
        "`a and b or c` could mean `(a and b) or c` or `a and (b or c)`.\nAdd brackets to say which one you mean.",
    )
}

pub fn not_needs_brackets(span: Span, written: &str) -> Diagnostic {
    // trace: decision.conditions.not
    fix(span, format!("not ({})", written), "`not` goes in front of one name, or of a test in brackets.")
}

pub fn needs_brackets_after(span: Span, word: &str, written: &str) -> Diagnostic {
    // trace: decision.lists.index_bare
    fix(
        span,
        format!("{} ({})", word, written),
        format!("After `{}`, one number or one name can stand alone.\nAnything more goes in brackets.", word),
    )
}

pub fn of_needs_name_or_brackets(span: Span, word: &str) -> Diagnostic {
    what(
        span,
        format!("after `{}`, put a `@variable`, a table, or something in brackets", word),
        format!("`{}` takes one name, or anything in brackets.", word),
    )
}

pub fn needs_of(span: Span, word: &str) -> Diagnostic {
    what(span, format!("`{}` needs `of` after it", word), format!("It's written like `{} of @scores`.", word))
}

pub fn round_needs_brackets(span: Span) -> Diagnostic {
    what(
        span,
        "`round` needs what it rounds in brackets, like `round (7 divided by 2)`",
        "Brackets show exactly what gets rounded.",
    )
}

pub fn remainder_needs_division(span: Span) -> Diagnostic {
    what(
        span,
        "`remainder of` needs a division in brackets, like `remainder of (7 divided by 2)`",
        "The remainder is what's left over after dividing.",
    )
}

pub fn show_needs_something(span: Span) -> Diagnostic {
    what(span, "`show` needs something to show", "Like `show \"Hello, World!\"`.")
}

pub fn else_without_if(span: Span) -> Diagnostic {
    what(span, "this `else` has no `if` before it", "`else` comes after an `if` block's lines, before `end if`.")
}

pub fn header_needs_colon(span: Span) -> Diagnostic {
    what(
        span,
        "this line needs to end with `:`",
        "A line that starts a block ends with `:`, and the block's lines come after it.",
    )
}

pub fn repeat_needs_times(span: Span) -> Diagnostic {
    what(
        span,
        "`repeat` needs `times`, `until` or `forever`",
        "Like `repeat 3 times:`, `repeat until @lives is 0:` or `repeat forever:`.",
    )
}

pub fn for_needs_each(span: Span) -> Diagnostic {
    what(span, "`for` needs `each` after it", "Like `for each @score in @scores:`.")
}

pub fn for_needs_in(span: Span) -> Diagnostic {
    what(span, "`for each` needs `in` and a table", "Like `for each @score in @scores:`.")
}

pub fn add_needs_to(span: Span) -> Diagnostic {
    what(span, "`add` needs `to` and a table", "Like `add 40 to @scores`.")
}

pub fn add_at_needs_position(span: Span) -> Diagnostic {
    what(span, "`at` needs `position` or `offset`", "Like `add 40 to @scores at position 2`.")
}

pub fn return_needs_value(span: Span) -> Diagnostic {
    what(
        span,
        "`return` needs the value to give back",
        "A procedure that gives nothing back ends with `finish` instead.",
    )
}

pub fn return_nothing_is_finish(span: Span) -> Diagnostic {
    // trace: decision.procedures.finish
    // trace: decision.procedures.return_nothing (replaced by `finish`, so `return nothing` gets a fix)
    fix(span, "finish", "A procedure that gives nothing back ends with `finish`.")
}

pub fn standalone_call_is_bare(span: Span, fixed: &str) -> Diagnostic {
    // trace: decision.procedures.nested_calls
    fix(span, fixed, "A call on its own line has no brackets. Brackets are for a call inside another line.")
}

pub fn nested_call_needs_brackets(span: Span, fixed: &str) -> Diagnostic {
    fix(span, fixed, "A call inside another line puts what it passes in brackets.")
}

pub fn end_of_is_end(span: Span) -> Diagnostic {
    // trace: decision.blocks.end
    // trace: decision.blocks.end_of (replaced by `end`, so `end_of` gets a fix)
    fix(span, "end", "A block ends with `end` and what it closes, like `end if` or `end procedure move`.")
}

pub fn wrong_end(span: Span, expected: &str) -> Diagnostic {
    fix(span, expected, "Each block ends by naming what it closes.")
}

pub fn end_names_wrong(span: Span, expected: &str) -> Diagnostic {
    fix(span, expected, "The `end` names the procedure it closes.")
}

pub fn no_end(span: Span, expected: &str) -> Diagnostic {
    what(
        span,
        format!("this block never ends: `{}` is missing", expected),
        "Every block ends with `end` and what it closes.",
    )
}

// ---- procedures ------------------------------------------------------------------------------------------------

pub fn procedure_needs_name(span: Span) -> Diagnostic {
    what(span, "this procedure needs a name", "Like `procedure move (@speed: small_whole) just finishes:`.")
}

pub fn procedure_name_has_at(span: Span, bare: &str) -> Diagnostic {
    fix(span, bare, "`@` marks variables. A procedure's name has no `@`.")
}

pub fn grammar_word_as_name(span: Span, word: &str) -> Diagnostic {
    // trace: decision.names.no_grammar_names
    what(
        span,
        format!("`{}` is one of TAKL's own words, so it can't be a name", word),
        format!("Each of TAKL's words keeps one meaning. Pick another name, like `{}_it`.", word),
    )
}

pub fn procedure_needs_ending(span: Span, name: &str) -> Diagnostic {
    // trace: decision.procedures.just_finishes
    what(
        span,
        format!("`{}` needs to say how it ends", name),
        "Write `returns` and a type, like `returns small_whole:`,\nor `just finishes:` if it gives nothing back.",
    )
}

pub fn params_split_by_bar(span: Span) -> Diagnostic {
    what(
        span,
        "params are split by `|`",
        "A comma joins a param's modifiers, like `@speed: small_whole, never_negative`.\nA `|` goes between params: `(@a: small_whole | @b: small_whole)`.",
    )
}

pub fn procedure_twice(span: Span, name: &str, first: Span) -> Diagnostic {
    what(span, format!("there's already a procedure called `{}`", name), "Each procedure needs its own name.")
        .because(first, format!("`{}` is first made here", name))
}

pub fn no_start_running(span: Span) -> Diagnostic {
    // trace: decision.procedures.start_running_required
    what(
        span,
        "this program has no `procedure start_running`",
        "Every program runs from `procedure start_running just finishes:`.\nOther languages call this `main`.",
    )
}

pub fn start_running_params_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "what `start_running`'s params would be given")
}

pub fn exit_code_type(span: Span, ty: &str) -> Diagnostic {
    // trace: decision.procedures.exit_code
    what(
        span,
        format!("`start_running` can't return a {}", ty),
        "`start_running` either `just finishes` or `returns tiny_whole, never_negative`:\nthe exit code, 0 to 255, which is what Mac and Linux keep.",
    )
}

pub fn needs_finish(span: Span, name: &str) -> Diagnostic {
    // trace: decision.procedures.finish
    what(
        span,
        format!("`{}` just finishes, so its last line is `finish`", name),
        "A procedure that gives nothing back ends with `finish`, on purpose.",
    )
}

pub fn needs_return(span: Span, name: &str, ty: &str) -> Diagnostic {
    what(
        span,
        format!("`{}` returns a {}, so it has to return one", name, ty),
        "Its last line is `return` and the value, like `return @total`.",
    )
}

pub fn never_runs(span: Span, word: &str) -> Diagnostic {
    // trace: decision.errors.return_nothing
    let left = if word == "stop" { "the loop" } else { "the procedure" };
    what(
        span,
        format!("this line never runs: `{}` already left {}", word, left),
        "Lines after it in the same block are never reached.",
    )
}

pub fn stop_outside_loop(span: Span) -> Diagnostic {
    what(
        span,
        "`stop` is outside every loop",
        "`stop` leaves the loop it's in. To leave a procedure that just finishes, use `finish`.",
    )
}

pub fn finish_ends_loop(span: Span) -> Diagnostic {
    fix(span, "stop", "`finish` leaves the whole procedure. `stop` leaves just the loop.")
}

pub fn finish_in_returning_procedure(span: Span, ty: &str) -> Diagnostic {
    what(
        span,
        format!("you said this gives back a {}, so return one", ty),
        "`finish` is for a procedure that `just finishes`. This one ends with `return` and a value.",
    )
}

pub fn return_in_finishing_procedure(span: Span, name: &str) -> Diagnostic {
    what(
        span,
        format!("`{}` just finishes, so it gives nothing back", name),
        "`return` always gives back a value. A procedure that just finishes ends with `finish`.",
    )
}

pub fn gives_nothing_back(span: Span, name: &str) -> Diagnostic {
    what(
        span,
        format!("`{}` gives nothing back, so it can't be used as a value", name),
        "It `just finishes`. Call it on its own line instead.",
    )
}

pub fn unknown_procedure(span: Span, name: &str, procedures: &[String]) -> Diagnostic {
    match near(name, procedures) {
        Some(close) => fix(span, close, format!("There's no procedure called `{}`.", name)),
        None => what(
            span,
            format!("there's no procedure called `{}`", name),
            "A procedure has to be made with `procedure` before it's called.",
        ),
    }
}

pub fn builtin_of_is_spaced(span: Span, word: &str) -> Diagnostic {
    // trace: decision.procedures.own_of
    fix(span, format!("{} of", word), format!("TAKL's own `{} of` is two words.", word))
}

pub fn too_many_args(span: Span, name: &str, count: usize) -> Diagnostic {
    what(
        span,
        format!("`{}` takes {} {}, and this is one too many", name, count, if count == 1 { "thing" } else { "things" }),
        "Each thing passed fills one of its params, in order.",
    )
}

pub fn missing_arg(span: Span, name: &str, number: usize) -> Diagnostic {
    what(
        span,
        format!("`{}` needs its param number {}", name, number),
        "Every param without a default has to be given something.",
    )
}

pub fn call_needs_share(span: Span, written: &str) -> Diagnostic {
    // trace: decision.procedures.share
    fix(
        span,
        format!("share {}", written),
        "This param says `share`, so the call says `share` too.\nThat way nothing gets changed without a word you can see.",
    )
}

pub fn param_not_shared(span: Span, name: &str) -> Diagnostic {
    what(
        span,
        format!("`{}` doesn't share this param, so it can't take `share`", name),
        "A param gets a copy unless it says `share`.",
    )
}

// ---- names and types -------------------------------------------------------------------------------------------

pub fn variable_needs_at(span: Span, word: &str) -> Diagnostic {
    // trace: decision.names.at_mark
    fix(span, format!("@{}", word), "Variables start with `@`.")
}

pub fn unknown_variable(span: Span, name: &str, names: &[String]) -> Diagnostic {
    match near(name, names) {
        Some(close) => fix(span, close, format!("There's no `{}` here.", name)),
        None => what(span, format!("there's no `{}` here", name), "A variable is made with `let` before it's used."),
    }
}

pub fn unknown_name(span: Span, name: &str, procedures: &[String], variables: &[String]) -> Diagnostic {
    let mut candidates: Vec<String> = procedures.to_vec();
    candidates.extend(variables.iter().map(|v| v.trim_start_matches('@').to_string()));
    match near(name, &candidates) {
        Some(close) if variables.contains(&format!("@{}", close)) => {
            fix(span, format!("@{}", close), format!("There's no `{}`.", name))
        }
        Some(close) => fix(span, close, format!("There's no `{}`.", name)),
        None => what(
            span,
            format!("there's no `{}`", name),
            "Variables start with `@`. Procedures are made with `procedure`.",
        ),
    }
}

pub fn used_after_block_not_designed(span: Span, name: &str, declared: Span) -> Diagnostic {
    not_designed(
        Stage::BeforeRunning,
        span,
        format!("whether `{}` can be used after the block it was made in ends", name),
    )
    .because(declared, format!("`{}` is made inside a block here", name))
}

pub fn name_twice_not_designed(span: Span, name: &str, first: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("making `{}` again while the first one is still there", name))
        .because(first, format!("`{}` is first made here", name))
}

pub fn type_is_not_a_value(span: Span, name: &str) -> Diagnostic {
    what(span, format!("`{}` is a type, not a value", name), "Types go after `:`, like `let @score: small_whole = 0`.")
}

pub fn unknown_type(span: Span, name: &str) -> Diagnostic {
    let certain = match name {
        "string" | "str" | "String" => Some(("text", "Other languages call text a string.")),
        "bool" | "boolean" | "Bool" | "Boolean" => Some(("true_or_false", "Other languages call this a boolean.")),
        "int8" | "i8" => Some(("tiny_whole", "TAKL names whole numbers by size. Other languages call this an int8.")),
        "int16" | "i16" => {
            Some(("small_whole", "TAKL names whole numbers by size. Other languages call this an int16."))
        }
        "int32" | "i32" => {
            Some(("medium_whole", "TAKL names whole numbers by size. Other languages call this an int32."))
        }
        "int64" | "i64" => Some(("big_whole", "TAKL names whole numbers by size. Other languages call this an int64.")),
        "uint8" | "u8" => {
            Some(("tiny_whole, never_negative", "TAKL names whole numbers by size. Other languages call this a uint8."))
        }
        "uint16" | "u16" => Some((
            "small_whole, never_negative",
            "TAKL names whole numbers by size. Other languages call this a uint16.",
        )),
        "uint32" | "u32" => Some((
            "medium_whole, never_negative",
            "TAKL names whole numbers by size. Other languages call this a uint32.",
        )),
        "uint64" | "u64" => {
            Some(("big_whole, never_negative", "TAKL names whole numbers by size. Other languages call this a uint64."))
        }
        _ => None,
    };
    if let Some((fixed, why)) = certain {
        return fix(span, fixed, why);
    }
    if matches!(name, "int" | "integer" | "long" | "short" | "uint" | "number") {
        return what(
            span,
            format!("TAKL has no `{}`", name),
            "TAKL names whole numbers by size: tiny_whole, small_whole, medium_whole, big_whole.\nAdd `, never_negative` for one that's never below zero.",
        );
    }
    if matches!(name, "float" | "double" | "f32" | "f64" | "decimal") {
        return what(
            span,
            format!("TAKL has no `{}`", name),
            "TAKL's decimals have sizes: tiny_decimal, small_decimal, medium_decimal, big_decimal.\nThey're exact. Add `, fast_but_inexact` for what other languages call a float.",
        );
    }
    match near(name, &crate::types::type_names()) {
        Some(close) => fix(span, close, format!("There's no type called `{}`.", name)),
        None => what(
            span,
            format!("there's no type called `{}`", name),
            "Types are text, true_or_false, and the sizes of whole, decimal and fraction, like small_whole.",
        ),
    }
}

pub fn expected_type(span: Span) -> Diagnostic {
    what(span, "a type goes here, like `small_whole` or `text`", "Every `let` says its type before its value.")
}

pub fn needs_type(span: Span, name: &str) -> Diagnostic {
    // trace: decision.syntax.let
    what(
        span,
        format!("`{}` needs a type, like `{}: small_whole`", name, name),
        "In TAKL, everything says its type first.",
    )
}

pub fn let_needs_value(span: Span, name: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("a `let` with no value, so `{}` would start as nothing", name))
}

pub fn table_needs_of(span: Span) -> Diagnostic {
    what(span, "`table` needs `of` and what it holds, like `table of small_whole`", "A table says what its items are.")
}

pub fn expected_modifier(span: Span) -> Diagnostic {
    what(
        span,
        "something like `can't_change` goes after the comma",
        "After a type, a comma adds a modifier: `can't_change`, `never_negative`, `fast_but_inexact`, `at least 0` or `at most 10`.",
    )
}

pub fn unknown_modifier(span: Span, word: &str) -> Diagnostic {
    let modifiers: Vec<String> =
        ["can't_change", "never_negative", "fast_but_inexact", "share"].iter().map(|m| m.to_string()).collect();
    match near(word, &modifiers) {
        Some(close) => fix(span, close, format!("There's no modifier called `{}`.", word)),
        None => what(
            span,
            format!("there's no modifier called `{}`", word),
            "Modifiers are `can't_change`, `never_negative`, `fast_but_inexact`, `at least`, `at most`, and `share` on a param.",
        ),
    }
}

pub fn fast_is_now(span: Span) -> Diagnostic {
    // trace: decision.types.fast_but_inexact
    fix(
        span,
        "fast_but_inexact",
        "A fast decimal is never exact, because computers store it in binary. The name says so.",
    )
}

pub fn normal_not_a_word(span: Span) -> Diagnostic {
    // trace: decision.types.exact_bare
    what(
        span,
        "`normal` isn't a modifier",
        "A decimal is exact unless it says `fast_but_inexact`, so there's nothing to add.",
    )
}

pub fn modifier_twice(span: Span) -> Diagnostic {
    what(span, "this modifier is already there", "Each modifier is written once.")
}

pub fn modifier_not_here(span: Span, word: &str, ty: &str) -> Diagnostic {
    what(
        span,
        format!("`{}` can't go on a {}", word, ty),
        "`never_negative` and bounds go on whole numbers, `fast_but_inexact` on decimals,\n`share` on a table param, and `can't_change` on a `let` or a param.",
    )
}

pub fn item_modifier_needs_brackets(span: Span, written: &str) -> Diagnostic {
    // trace: decision.tables.item_modifiers
    let fixed = match written.rfind("of ") {
        Some(index) => format!("{}({})", &written[..index + 3], &written[index + 3..]),
        None => written.to_string(),
    };
    let start = Span::new(span.start, span.end);
    fix(start, fixed, "Is that modifier the table's or its items'? Brackets say: inside them is what each item is.")
}

pub fn share_goes_on_value(span: Span) -> Diagnostic {
    what(
        span,
        "`share` goes on the value, like `= share @scores`",
        "`let @b: table of small_whole = share @a` makes `@b` the same table as `@a`.",
    )
}

pub fn share_on_non_table_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "sharing something that isn't a table")
}

pub fn bound_needs_number(span: Span) -> Diagnostic {
    what(
        span,
        "a bound needs a whole number, like `at least 0`",
        "Bounds say the smallest or biggest a variable may be.",
    )
}

pub fn bound_outside_type(span: Span, ty: &str) -> Diagnostic {
    what(span, format!("this bound is outside what a {} holds", ty), "A bound has to be a number the type can hold.")
}

pub fn bounds_crossed(span: Span) -> Diagnostic {
    what(
        span,
        "`at least` is bigger than `at most`, so no number fits",
        "The smallest a variable may be can't be bigger than the biggest.",
    )
}

pub fn bounds_on_decimals_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "bounds on decimals and fractions")
}

pub fn bounds_here_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "bounds here, on a table's items or a procedure's return")
}

pub fn wrong_type(span: Span, got: &str, want: &str) -> Diagnostic {
    what(span, format!("this is a {}, but a {} goes here", got, want), "A value has to match the type it's going into.")
}

pub fn needs_test(span: Span, got: &str) -> Diagnostic {
    what(span, format!("this is a {}, but a test goes here", got), "A test is true or false, like `@lives is 0`.")
}

pub fn needs_number(span: Span, got: &str) -> Diagnostic {
    what(
        span,
        format!("this is a {}, but maths needs a number", got),
        "`plus`, `minus`, `times` and `divided by` work on numbers.",
    )
}

pub fn needs_whole_number(span: Span, got: &str) -> Diagnostic {
    what(
        span,
        format!("this is a {}, but a whole number goes here", got),
        "Counting and places in a table use whole numbers.",
    )
}

pub fn mixing_types_not_designed(span: Span, a: &str, b: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("maths that mixes a {} with a {}", a, b))
}

pub fn comparing_types_not_designed(span: Span, a: &str, b: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("comparing a {} with a {}", a, b))
}

pub fn ordering_not_designed(span: Span, ty: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("whether one {} is more or less than another", ty))
}

pub fn comparing_tables_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "whether two tables are the same")
}

pub fn joining_text_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "joining text with `plus`")
}

pub fn fraction_maths_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "maths on fractions")
}

pub fn about_needs_fast(span: Span) -> Diagnostic {
    // trace: decision.types.approx
    what(
        span,
        "`~` is only for `fast_but_inexact` decimals",
        "`~0.1` means \"the nearest a fast decimal can hold\". Exact decimals hold what you write.",
    )
}

pub fn round_this_not_designed(span: Span, ty: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("rounding a {}", ty))
}

pub fn remainder_needs_wholes_not_designed(span: Span, ty: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("the remainder of a {}", ty))
}

pub fn show_true_or_false_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "how `show` prints true or false")
}

pub fn show_table_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "how `show` prints a table")
}

pub fn changing_loop_item_not_designed(span: Span, name: &str) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, format!("changing `{}` while `for each` goes through it", name))
}

pub fn cant_change(span: Span, name: &str, declared: Span) -> Diagnostic {
    // trace: decision.lets.change
    what(span, format!("`{}` can't change", name), "It says `can't_change`, so it keeps the value it started with.")
        .because(declared, format!("`{}` is made can't_change here", name))
}

// ---- tables ----------------------------------------------------------------------------------------------------

pub fn block_needs_table(span: Span, ty: &str) -> Diagnostic {
    what(
        span,
        format!("a block of items needs a table, but this is a {}", ty),
        "Only a table can be written as a block, one item a line.",
    )
}

pub fn needs_key(span: Span) -> Diagnostic {
    what(
        span,
        "this table's items have names, so this needs one, like `\"sam\": 10`",
        "A table of `text to small_whole` keeps each item under a name.",
    )
}

pub fn no_keys_here(span: Span) -> Diagnostic {
    what(span, "this table is numbered, so its items don't take names", "A numbered table is keyed 1, 2, 3 by itself.")
}

pub fn keyed_table_needs_type_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "a table with named keys where its type isn't written")
}

pub fn table_mixes_types(span: Span) -> Diagnostic {
    what(span, "this table's items aren't all the same type", "Every item in a table has the type the table says.")
}

pub fn add_needs_table(span: Span, name: &str, ty: &str) -> Diagnostic {
    what(span, format!("`{}` is a {}, not a table", name, ty), "`add` puts an item into a table.")
}

pub fn at_on_keyed_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "putting a named item at a place in the table")
}

pub fn for_each_needs_table(span: Span, ty: &str) -> Diagnostic {
    what(span, format!("`for each` goes through a table, but this is a {}", ty), "Like `for each @score in @scores:`.")
}

pub fn for_each_shape_not_designed(span: Span) -> Diagnostic {
    not_designed(
        Stage::BeforeRunning,
        span,
        "this shape of `for each`: one name for a table with named keys, or two for a numbered one",
    )
}

pub fn index_needs_table(span: Span, ty: &str) -> Diagnostic {
    what(span, format!("this is a {}, so it has no positions", ty), "`position` and `offset` work on tables and text.")
}

pub fn reading_named_key_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "how to read one item of a table with named keys")
}

pub fn of_keyed_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::BeforeRunning, span, "this `of` on a table with named keys")
}

pub fn of_needs_numbers(span: Span, ty: &str) -> Diagnostic {
    what(
        span,
        format!("this is a {}, but it needs a table of numbers", ty),
        "`sum of`, `biggest of`, `smallest of` and `average of` work on a table of numbers.",
    )
}

pub fn would_leave_hole(span: Span, table: &str, length: usize, written: &str) -> Diagnostic {
    // trace: decision.tables.no_holes
    Diagnostic::while_running(
        span,
        Headline::What(format!(
            "{} would leave a hole: {} has {} {}",
            written,
            table,
            length,
            if length == 1 { "item" } else { "items" }
        )),
        "A table never has holes. Use `add` without `at` to put an item on the end.",
    )
}

pub fn index_outside(span: Span, written: &str, count: usize, unit: &str) -> Diagnostic {
    let unit = if count == 1 { unit.trim_end_matches('s') } else { unit };
    Diagnostic::while_running(
        span,
        Headline::What(format!("there's no {}: it only has {} {}", written, count, unit)),
        "`position` counts from 1, and `offset` counts from 0.",
    )
}

pub fn key_already_there_not_designed(span: Span, key: &str) -> Diagnostic {
    not_designed(Stage::WhileRunning, span, format!("adding {} again, when it's already in the table", key))
}

pub fn no_items(span: Span, word: &str) -> Diagnostic {
    Diagnostic::while_running(
        span,
        Headline::What(format!("the table is empty, so it has no {}", word)),
        format!("`{} of` needs at least one item.", word),
    )
}

// ---- numbers ---------------------------------------------------------------------------------------------------

/// trace: decision.numbers.range
pub fn too_big_for(
    span: Span,
    value: &str,
    ty: &str,
    least: &BigInt,
    most: &BigInt,
    size: Size,
    before: bool,
) -> Diagnostic {
    let never_negative = ty.contains("never_negative");
    let direction = if value.starts_with('-') { "too small" } else { "too big" };
    let other = other_name(size, never_negative);
    let article = if other.starts_with('u') { "a" } else { "an" };
    let why = format!("A {} holds {} to {}. Other languages call this {} {}.", ty, least, most, article, other);
    let headline = format!("{} is {} for a {}", value, direction, ty);
    if before { what(span, headline, why) } else { Diagnostic::while_running(span, Headline::What(headline), why) }
}

pub fn outside_bounds(
    span: Span,
    value: &str,
    least: &Option<BigInt>,
    most: &Option<BigInt>,
    before: bool,
) -> Diagnostic {
    // trace: decision.types.bounds
    let rule = match (least, most) {
        (Some(least), Some(most)) => format!("at least {} and at most {}", least, most),
        (Some(least), None) => format!("at least {}", least),
        (None, Some(most)) => format!("at most {}", most),
        (None, None) => String::new(),
    };
    let headline = format!("{} is outside this variable's bounds", value);
    let why = format!("It has to be {}.", rule);
    if before { what(span, headline, why) } else { Diagnostic::while_running(span, Headline::What(headline), why) }
}

pub fn uneven_whole_division(span: Span, written: &str, stage: Stage) -> Diagnostic {
    // trace: decision.maths.whole_divide
    at(
        stage,
        span,
        format!("{} doesn't come out even, so a whole number can't hold it", written),
        "Whole numbers never drop what's left over on their own.\nUse `round (...)` to round it, a decimal or a fraction to keep it,\nor `remainder of (...)` for what's left over.",
    )
}

pub fn divided_by_zero(span: Span, stage: Stage) -> Diagnostic {
    at(stage, span, "this divides by zero", "Nothing can be divided by zero.")
}

pub fn round_halfway_not_designed(span: Span, stage: Stage) -> Diagnostic {
    not_designed(stage, span, "which way `round` goes when the answer is exactly halfway")
}

pub fn negative_remainder_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::WhileRunning, span, "the remainder when a number is below zero")
}

pub fn negative_repeat_not_designed(span: Span, count: &str) -> Diagnostic {
    not_designed(Stage::WhileRunning, span, format!("repeating {} times", count))
}

pub fn decimal_trouble(span: Span, trouble: Trouble, ty: &str, stage: Stage) -> Diagnostic {
    // trace: decision.decimals.rounding
    match trouble {
        Trouble::GoesOnForever => at(
            stage,
            span,
            "this goes on forever, so a decimal can't hold it exactly",
            "Exact decimals never round on their own. Round it with `round (...)`, or use a fraction.",
        ),
        Trouble::TooManyDigits => at(
            stage,
            span,
            format!("this has more digits than a {} holds", ty),
            "Exact decimals never round on their own. Round it with `round (...)`, or use a bigger decimal.",
        ),
        Trouble::TooBig => at(stage, span, format!("this is too big for a {}", ty), "Use a bigger decimal."),
        Trouble::TooSmall => at(
            stage,
            span,
            format!("this is too close to zero for a {} to hold exactly", ty),
            "Exact decimals never round on their own. Round it with `round (...)`, or use a bigger decimal.",
        ),
        Trouble::DividedByZero => divided_by_zero(span, stage),
        Trouble::Halfway => round_halfway_not_designed(span, stage),
    }
}

pub fn fast_cant_hold(span: Span, written: &str, nearest: &str, ty: &str, stage: Stage) -> Diagnostic {
    // trace: decision.types.approx
    at(
        stage,
        span,
        format!("a {} can't hold {} exactly", ty, written),
        format!(
            "The nearest it holds is {}.\nWrite `~{}` for \"about {}\". It's inexact because computers store these in binary.",
            nearest, written, written
        ),
    )
}

pub fn fast_too_big(span: Span, written: &str, ty: &str, stage: Stage) -> Diagnostic {
    at(stage, span, format!("{} is too big for a {}", written, ty), "Use a bigger decimal.")
}

pub fn fraction_too_big(span: Span, written: &str, ty: &str, size: Size, stage: Stage) -> Diagnostic {
    at(
        stage,
        span,
        format!("{} is too big for a {}", written, ty),
        format!("The top and the bottom of a {} are each a {}_whole.", ty, size.word()),
    )
}

pub fn fraction_not_lowest_not_designed(span: Span, written: &str) -> Diagnostic {
    not_designed(
        Stage::WhileRunning,
        span,
        format!("whether `show` prints {} as it is or in its lowest terms", written),
    )
}

pub fn too_deep_not_designed(span: Span) -> Diagnostic {
    not_designed(Stage::WhileRunning, span, "what happens when procedures call each other this deep (10000 calls)")
}
