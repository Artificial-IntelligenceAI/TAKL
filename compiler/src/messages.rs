//! Every word a kid reads in an error, each one copied from a golden test's `.out`.
//!
//! trace: decision.errors.layout
//!
//! Wording is design, so nothing here is made up. A mistake no golden test shows yet gets the shape of a
//! `NotDesignedYet` message from `not_designed_yet`, and the question goes to design claude.

use crate::failure::Failure;
use crate::source::Span;

const NOT_DESIGNED_WHY: &str =
    "TAKL hasn't decided how this works yet, so it stops instead of guessing.\nIt's on TAKL's list of open questions.";

// trace: decision.house.impl_rules
/// Something TAKL hasn't decided, so it stops instead of guessing.
pub fn not_designed_yet(span: Span, what: impl Into<String>) -> Failure {
    Failure::what(span, what, NOT_DESIGNED_WHY)
}

// trace: decision.maths.words
pub fn maths_symbol(span: Span, symbol: char) -> Failure {
    match symbol {
        '+' => Failure::fix(span, "plus", "TAKL writes maths in words."),
        '*' => Failure::fix(span, "times", "TAKL writes maths in words."),
        '/' => Failure::fix(span, "divided by", "TAKL writes maths in words, and `/` usually means \"or\"."),
        _ => Failure::fix(span, "minus", "TAKL writes maths in words."),
    }
}

// trace: decision.conditions.isnt
pub fn is_not(span: Span) -> Failure {
    Failure::fix(span, "isn't", "TAKL writes \"is not\" as one word, `isn't`.")
}

// trace: decision.conditions.not
pub fn not_needs_brackets(span: Span, fix: String) -> Failure {
    Failure::fix(span, fix, "`not` goes in front of one name, or of a test in brackets.")
}

// trace: decision.conditions.and_or
pub fn and_with_or(span: Span) -> Failure {
    Failure::what(
        span,
        "this mixes `and` with `or` without brackets",
        "`a and b or c` could mean `(a and b) or c` or `a and (b or c)`.\nAdd brackets to say which one you mean.",
    )
}

pub fn does_not_belong(span: Span, text: &str) -> Failure {
    Failure::what(span, format!("`{}` doesn't belong here", text), "TAKL couldn't read the line from this point on.")
}

// trace: decision.lists.index_bare
pub fn needs_brackets(span: Span, word: &str, fix: String) -> Failure {
    Failure::fix(
        span,
        fix,
        format!("After `{}`, one number or one name can stand alone.\nAnything more goes in brackets.", word),
    )
}

// trace: decision.loops.forever
pub fn stop_outside_loops(span: Span) -> Failure {
    Failure::what(
        span,
        "`stop` is outside every loop",
        "`stop` leaves the loop it's in. To leave a procedure that just finishes, use `finish`.",
    )
}

// trace: decision.errors.return_nothing
pub fn never_runs(span: Span, left: &str) -> Failure {
    let what = if left == "stop" {
        "this line never runs: `stop` already left the loop".to_string()
    } else {
        format!("this line never runs: `{}` already left the procedure", left)
    };
    Failure::what(span, what, "Lines after it in the same block are never reached.")
}

// trace: decision.numbers.range
pub fn too_big(span: Span, value: &str, type_name: &str, holds: &str, other_name: &str) -> Failure {
    Failure::what(
        span,
        format!("{} is too big for a {}", value, type_name),
        format!("A {} holds {}. Other languages call this {}.", type_name, holds, other_name),
    )
}

pub fn too_small(span: Span, value: &str, type_name: &str, holds: &str, other_name: &str) -> Failure {
    Failure::what(
        span,
        format!("{} is too small for a {}", value, type_name),
        format!("A {} holds {}. Other languages call this {}.", type_name, holds, other_name),
    )
}

// trace: decision.spelling.typos
pub fn unknown_variable_near(span: Span, written: &str, near: &str) -> Failure {
    Failure::fix(span, format!("@{}", near), format!("There's no `@{}` here.", written))
}

pub fn unknown_variable(span: Span, written: &str) -> Failure {
    Failure::what(span, format!("there's no `@{}` here", written), "A variable is made with `let` before it's used.")
}

// trace: decision.names.at_mark
pub fn needs_at(span: Span, name: &str) -> Failure {
    Failure::fix(span, format!("@{}", name), "Variables start with `@`.")
}

// trace: decision.names.no_grammar_names
pub fn grammar_word_as_name(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` is one of TAKL's own words, so it can't be a name", name),
        format!("Each of TAKL's words keeps one meaning. Pick another name, like `{}_it`.", name),
    )
}

// trace: decision.blocks.end
pub fn end_names(span: Span, name: &str) -> Failure {
    Failure::fix(span, format!("end procedure {}", name), "The `end` names the procedure it closes.")
}

// trace: decision.procedures.exit_code
pub fn exit_code_type(span: Span, type_name: &str) -> Failure {
    Failure::what(
        span,
        format!("`start_running` can't return a {}", type_name),
        "`start_running` either `just finishes` or `returns tiny_whole, never_negative`:\nthe exit code, 0 to 255, which is what Mac and Linux keep.",
    )
}

// trace: decision.procedures.finish
pub fn must_finish(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` just finishes, so its last line is `finish`", name),
        "A procedure that gives nothing back ends with `finish`, on purpose.",
    )
}

pub fn finish_in_loop(span: Span) -> Failure {
    Failure::fix(span, "stop", "`finish` leaves the whole procedure. `stop` leaves just the loop.")
}

pub fn finish_in_returns(span: Span, type_name: &str) -> Failure {
    Failure::what(
        span,
        format!("you said this gives back a {}, so return one", type_name),
        "`finish` is for a procedure that `just finishes`. This one ends with `return` and a value.",
    )
}

// trace: decision.procedures.just_finishes
pub fn needs_ending(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` needs to say how it ends", name),
        "Write `returns` and a type, like `returns small_whole:`,\nor `just finishes:` if it gives nothing back.",
    )
}

pub fn needs_return(span: Span, name: &str, type_name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` returns a {}, so it has to return one", name, type_name),
        "Its last line is `return` and the value, like `return @total`.",
    )
}

// trace: decision.procedures.defaults
pub fn missing_argument(span: Span, name: &str, number: usize) -> Failure {
    Failure::what(
        span,
        format!("`{}` needs its param number {}", name, number),
        "Every param without a default has to be given something.",
    )
}

pub fn one_too_many(span: Span, name: &str, takes: usize) -> Failure {
    Failure::what(
        span,
        format!("`{}` takes {} things, and this is one too many", name, takes),
        "Each thing passed fills one of its params, in order.",
    )
}

// trace: decision.procedures.nested_calls
pub fn call_needs_brackets(span: Span, fix: String) -> Failure {
    Failure::fix(span, fix, "A call inside another line puts what it passes in brackets.")
}

pub fn call_without_brackets(span: Span, fix: String) -> Failure {
    Failure::fix(span, fix, "A call on its own line has no brackets. Brackets are for a call inside another line.")
}

pub fn gives_nothing_back(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` gives nothing back, so it can't be used as a value", name),
        "It `just finishes`. Call it on its own line instead.",
    )
}

// trace: decision.procedures.own_of
pub fn built_in_two_words(span: Span, family: &str) -> Failure {
    Failure::fix(span, format!("{} of", family), format!("TAKL's own `{} of` is two words.", family))
}

pub fn unknown_procedure_near(span: Span, written: &str, near: &str) -> Failure {
    Failure::fix(span, near, format!("There's no procedure called `{}`.", written))
}

pub fn unknown_procedure(span: Span, written: &str) -> Failure {
    Failure::what(
        span,
        format!("there's no procedure called `{}`", written),
        "A procedure has to be made with `procedure` before it's called.",
    )
}

// trace: decision.procedures.finish
pub fn return_in_finishing(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("`{}` just finishes, so it gives nothing back", name),
        "`return` always gives back a value. A procedure that just finishes ends with `finish`.",
    )
}

pub fn return_nothing(span: Span) -> Failure {
    Failure::fix(span, "finish", "A procedure that gives nothing back ends with `finish`.")
}

// trace: decision.procedures.share
pub fn share_at_call(span: Span, name: &str) -> Failure {
    Failure::fix(
        span,
        format!("share @{}", name),
        "This param says `share`, so the call says `share` too.\nThat way nothing gets changed without a word you can see.",
    )
}

// trace: decision.procedures.start_running_required
pub fn no_start_running(span: Span) -> Failure {
    Failure::what(
        span,
        "this program has no `procedure start_running`",
        "Every program runs from `procedure start_running just finishes:`.\nOther languages call this `main`.",
    )
}

// trace: decision.spelling.typos
pub fn unknown_modifier_near(span: Span, written: &str, near: &str) -> Failure {
    Failure::fix(span, near, format!("There's no modifier called `{}`.", written))
}

// trace: decision.blocks.end
pub fn end_of(span: Span) -> Failure {
    Failure::fix(span, "end", "A block ends with `end` and what it closes, like `end if` or `end procedure move`.")
}

// trace: decision.syntax.no_semicolons
pub fn semicolon(span: Span) -> Failure {
    Failure::what(span, "TAKL lines don't end with `;`", "A line ends where the line ends. Take the `;` out.")
}

// trace: decision.procedures.start_running_required
pub fn outside_procedures(span: Span) -> Failure {
    Failure::what(
        span,
        "this line is outside every procedure, so it would never run",
        "A program runs from `procedure start_running`, and every line lives inside a procedure.",
    )
}

// trace: decision.tables.item_modifiers
pub fn item_modifier(span: Span, fix: String) -> Failure {
    Failure::fix(
        span,
        fix,
        "Is that modifier the table's or its items'? Brackets say: inside them is what each item is.",
    )
}

// trace: decision.tables.shape
pub fn square_not_closed(span: Span) -> Failure {
    Failure::what(
        span,
        "this `[` is never closed on its line",
        "A table in `[ ]` stays on one line. For a longer one, make it a block:\nlet @level: table of small_whole:\n  10\n  20\nend table @level",
    )
}

// trace: decision.values.empty
pub fn empty_text(span: Span) -> Failure {
    Failure::fix(span, "empty", "Empty text is written `empty`, so it looks like you meant it.")
}

pub fn empty_table(span: Span) -> Failure {
    Failure::fix(span, "empty", "An empty table is written `empty`, so it looks like you meant it.")
}

// trace: decision.types.approx
pub fn cannot_hold_exactly(span: Span, type_name: &str, written: &str, nearest: &str) -> Failure {
    Failure::what(
        span,
        format!("a {} can't hold {} exactly", type_name, written),
        format!(
            "The nearest it holds is {}.\nWrite `~{}` for \"about {}\". It's inexact because computers store these in binary.",
            nearest, written, written
        ),
    )
}

pub fn about_on_exact(span: Span) -> Failure {
    Failure::what(
        span,
        "`~` is only for `fast_but_inexact` decimals",
        "`~0.1` means \"the nearest a fast decimal can hold\". Exact decimals hold what you write.",
    )
}

// trace: decision.types.bounds
pub fn outside_bounds(span: Span, value: &str, rule: &str) -> Failure {
    Failure::what(span, format!("{} is outside this variable's bounds", value), format!("It has to be {}.", rule))
}

pub fn bounds_crossed(span: Span) -> Failure {
    Failure::what(
        span,
        "`at least` is bigger than `at most`, so no number fits",
        "The smallest a variable may be can't be bigger than the biggest.",
    )
}

pub fn bound_outside_type(span: Span, type_name: &str) -> Failure {
    Failure::what(
        span,
        format!("this bound is outside what a {} holds", type_name),
        "A bound has to be a number the type can hold.",
    )
}

// trace: decision.decimals.rounding
pub fn too_many_digits(span: Span, type_name: &str) -> Failure {
    Failure::what(
        span,
        format!("this has more digits than a {} holds", type_name),
        "Exact decimals never round on their own. Round it with `round (...)`, or use a bigger decimal.",
    )
}

pub fn goes_on_forever(span: Span) -> Failure {
    Failure::what(
        span,
        "this goes on forever, so a decimal can't hold it exactly",
        "Exact decimals never round on their own. Round it with `round (...)`, or use a fraction.",
    )
}

// trace: decision.types.exact_bare
pub fn normal_modifier(span: Span) -> Failure {
    Failure::what(
        span,
        "`normal` isn't a modifier",
        "A decimal is exact unless it says `fast_but_inexact`, so there's nothing to add.",
    )
}

// trace: decision.types.fast_but_inexact
pub fn fast_is_now(span: Span) -> Failure {
    Failure::fix(
        span,
        "fast_but_inexact",
        "A fast decimal is never exact, because computers store it in binary. The name says so.",
    )
}

pub fn fast_too_big_written(span: Span, value: &str, type_name: &str) -> Failure {
    Failure::what(span, format!("{} is too big for a {}", value, type_name), "Use a bigger decimal.")
}

pub fn fast_too_big(span: Span, type_name: &str) -> Failure {
    Failure::what(span, format!("this is too big for a {}", type_name), "Use a bigger decimal.")
}

// trace: decision.types.over
pub fn fraction_too_big(span: Span, value: &str, type_name: &str, whole_name: &str) -> Failure {
    Failure::what(
        span,
        format!("{} is too big for a {}", value, type_name),
        format!("The top and the bottom of a {} are each a {}.", type_name, whole_name),
    )
}

// trace: decision.types.whole
pub fn no_int(span: Span, word: &str) -> Failure {
    Failure::what(
        span,
        format!("TAKL has no `{}`", word),
        "TAKL names whole numbers by size: tiny_whole, small_whole, medium_whole, big_whole.\nAdd `, never_negative` for one that's never below zero.",
    )
}

// trace: decision.text.name
pub fn string_is_text(span: Span) -> Failure {
    Failure::fix(span, "text", "Other languages call text a string.")
}

pub fn unknown_type_near(span: Span, written: &str, near: &str) -> Failure {
    Failure::fix(span, near, format!("There's no type called `{}`.", written))
}

pub fn bits_name(span: Span, fix: &str, other_name: &str) -> Failure {
    Failure::fix(span, fix, format!("TAKL names whole numbers by size. Other languages call this {}.", other_name))
}

pub fn wrong_type(span: Span, is: &str, goes: &str) -> Failure {
    Failure::what(
        span,
        format!("this is {}, but a {} goes here", is, goes),
        "A value has to match the type it's going into.",
    )
}

// trace: decision.conditions.if
pub fn needs_test(span: Span, is: &str) -> Failure {
    Failure::what(span, format!("this is {}, but a test goes here", is), "A test is true or false, like `@lives is 0`.")
}

// trace: decision.lets.change
pub fn cant_change(span: Span, name: &str, made: Span) -> Failure {
    Failure::what(
        span,
        format!("`@{}` can't change", name),
        "It says `can't_change`, so it keeps the value it started with.",
    )
    .because(format!("`@{}` is made can't_change here", name), made)
}

// trace: decision.lists.index
pub fn no_such_place(span: Span, counting: &str, asked: &str, count: usize, of_what: &str) -> Failure {
    let things = if count == 1 { of_what.trim_end_matches('s').to_string() } else { of_what.to_string() };
    Failure::what(
        span,
        format!("there's no {} {}: it only has {} {}", counting, asked, count, things),
        "`position` counts from 1, and `offset` counts from 0.",
    )
}

// trace: decision.tables.no_holes
pub fn would_leave_hole(span: Span, counting: &str, asked: &str, table: &str, count: usize) -> Failure {
    Failure::what(
        span,
        format!("{} {} would leave a hole: @{} has {} items", counting, asked, table, count),
        "A table never has holes. Use `add` without `at` to put an item on the end.",
    )
}

// trace: decision.maths.whole_divide
pub fn divides_by_zero(span: Span) -> Failure {
    Failure::what(span, "this divides by zero", "Nothing can be divided by zero.")
}

pub fn uneven(span: Span, top: &str, bottom: &str) -> Failure {
    Failure::what(
        span,
        format!("{} divided by {} doesn't come out even, so a whole number can't hold it", top, bottom),
        "Whole numbers never drop what's left over on their own.\nUse `round (...)` to round it, a decimal or a fraction to keep it,\nor `remainder of (...)` for what's left over.",
    )
}

// trace: decision.tables.of_family
pub fn no_items(span: Span, family: &str) -> Failure {
    Failure::what(
        span,
        format!("the table is empty, so it has no {}", family),
        format!("`{} of` needs at least one item.", family),
    )
}

/// A piece of maths that can't give an answer, said for the type it was working in.
pub fn problem(problem: crate::numbers::Problem, span: Span, ty: &crate::types::Type) -> Failure {
    use crate::numbers::Problem;
    use crate::types::Type;
    match problem {
        Problem::TooBig(value) => match ty {
            Type::Whole(whole) => too_big(span, &value.to_string(), &whole.name(), &whole.holds(), &whole.other_name()),
            _ => fast_too_big(span, &ty.name()),
        },
        Problem::TooSmall(value) => match ty {
            Type::Whole(whole) => {
                too_small(span, &value.to_string(), &whole.name(), &whole.holds(), &whole.other_name())
            }
            _ => fast_too_big(span, &ty.name()),
        },
        Problem::DividesByZero => divides_by_zero(span),
        Problem::Uneven(top, bottom) => uneven(span, &top.to_string(), &bottom.to_string()),
        Problem::GoesOnForever => goes_on_forever(span),
        Problem::TooManyDigits => too_many_digits(span, &ty.name()),
        Problem::Halfway => not_designed_yet(span, "which way `round` goes when the answer is exactly halfway"),
        Problem::FastTooBig => fast_too_big(span, &ty.name()),
        Problem::NegativeRemainder => not_designed_yet(span, "the remainder when a number is below zero"),
    }
}

// trace: decision.blocks.end
pub fn no_end(span: Span, closer: &str) -> Failure {
    Failure::what(
        span,
        format!("this block never ends: `{}` is missing", closer),
        "Every block ends with `end` and what it closes.",
    )
}

pub fn wrong_end(span: Span, fix: String) -> Failure {
    Failure::fix(span, fix, "Each block ends by naming what it closes.")
}

pub fn bracket_not_closed(span: Span) -> Failure {
    Failure::what(span, "this `(` is never closed", "Every `(` needs a `)` on the same line.")
}

// trace: decision.show.pieces
pub fn show_nothing(span: Span) -> Failure {
    Failure::what(span, "`show` needs something to show", "Like `show \"Hello, World!\"`.")
}

// trace: decision.text.quotes
pub fn text_not_closed(span: Span) -> Failure {
    Failure::what(span, "this text has no closing `\"`", "Text starts with `\"` and ends with `\"` on the same line.")
}

// trace: decision.names.at_mark
pub fn bare_unknown(span: Span, name: &str) -> Failure {
    Failure::what(
        span,
        format!("there's no `{}`", name),
        "Variables start with `@`. Procedures are made with `procedure`.",
    )
}

// trace: decision.decimals.rounding
pub fn round_needs_brackets(span: Span) -> Failure {
    Failure::what(
        span,
        "`round` needs what it rounds in brackets, like `round (7 divided by 2)`",
        "Brackets show exactly what gets rounded.",
    )
}

// trace: decision.maths.remainder
pub fn remainder_needs_division(span: Span) -> Failure {
    Failure::what(
        span,
        "`remainder of` needs a division in brackets, like `remainder of (7 divided by 2)`",
        "The remainder is what's left over after dividing.",
    )
}

// trace: decision.values.empty
pub fn empty_no_type(span: Span) -> Failure {
    Failure::what(
        span,
        "`empty` doesn't know what it's empty of here",
        "`empty` works where the type is known, like `let @name: text = empty`.",
    )
}

// trace: decision.types.decimals
pub fn no_float(span: Span, word: &str) -> Failure {
    Failure::what(
        span,
        format!("TAKL has no `{}`", word),
        "TAKL's decimals have sizes: tiny_decimal, small_decimal, medium_decimal, big_decimal.\nThey're exact. Add `, fast_but_inexact` for what other languages call a float.",
    )
}

// trace: decision.types.true_or_false
pub fn bool_is_true_or_false(span: Span) -> Failure {
    Failure::fix(span, "true_or_false", "Other languages call this a boolean.")
}
