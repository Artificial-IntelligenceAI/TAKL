//! TAKL's types, as the checker and every tier see them.
//!
//! trace: decision.types.whole
//! trace: decision.types.true_or_false
//! trace: decision.text.name

use crate::numbers::{Size, WholeType};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Whole(WholeType),
    Decimal {
        size: Size,
        fast: bool,
    },
    Fraction(Size),
    Text,
    TrueOrFalse,
    /// `table of T`, keyed 1, 2, 3, or `table of text to T` when `keyed`.
    Table {
        keyed: bool,
        item: Box<Type>,
    },
    /// Made only of written numbers, worked out exactly with no size until it goes somewhere typed.
    WrittenWhole,
    WrittenDecimal,
    WrittenFraction,
    /// `empty` before anything says what it's empty of.
    WrittenEmpty,
    /// What a procedure that just finishes gives back.
    Nothing,
    /// Couldn't be worked out; already reported, so nothing more is said about it.
    Broken,
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::Whole(whole) => whole.name(),
            Type::Decimal { size, fast: false } => format!("{}_decimal", size.word()),
            Type::Decimal { size, fast: true } => format!("{}_decimal, fast_but_inexact", size.word()),
            Type::Fraction(size) => format!("{}_fraction", size.word()),
            Type::Text => "text".to_string(),
            Type::TrueOrFalse => "true_or_false".to_string(),
            Type::Table { keyed: false, item } => format!("table of {}", item.name()),
            Type::Table { keyed: true, item } => format!("table of text to {}", item.name()),
            Type::WrittenWhole => "whole number".to_string(),
            Type::WrittenDecimal => "decimal".to_string(),
            Type::WrittenFraction => "fraction".to_string(),
            Type::WrittenEmpty => "empty".to_string(),
            Type::Nothing => "nothing".to_string(),
            Type::Broken => "?".to_string(),
        }
    }

    /// The name with its article, for "this is a ..., but".
    pub fn with_article(&self) -> String {
        format!("a {}", self.name())
    }

    pub fn is_broken(&self) -> bool {
        match self {
            Type::Broken => true,
            Type::Table { item, .. } => item.is_broken(),
            _ => false,
        }
    }

    pub fn is_written(&self) -> bool {
        match self {
            Type::WrittenWhole | Type::WrittenDecimal | Type::WrittenFraction | Type::WrittenEmpty => true,
            Type::Table { item, .. } => item.is_written(),
            _ => false,
        }
    }

    pub fn is_number(&self) -> bool {
        matches!(
            self,
            Type::Whole(_)
                | Type::Decimal { .. }
                | Type::Fraction(_)
                | Type::WrittenWhole
                | Type::WrittenDecimal
                | Type::WrittenFraction
        )
    }

    /// Held in the heap and counted, rather than in a register's bits.
    pub fn is_heap(&self) -> bool {
        match self {
            // A float128 doesn't fit a register's 64 bits, so it is held in the heap too.
            Type::Decimal { size: Size::Big, fast: true } => true,
            Type::Decimal { fast: false, .. } | Type::Fraction(_) | Type::Text | Type::Table { .. } => true,
            _ => false,
        }
    }

    /// Written in a program as the types after `:`.
    pub fn named(word: &str) -> Option<Type> {
        if word == "text" {
            return Some(Type::Text);
        }
        if word == "true_or_false" {
            return Some(Type::TrueOrFalse);
        }
        let (size, kind) = word.split_once('_')?;
        let size = Size::from_word(size)?;
        match kind {
            "whole" => Some(Type::Whole(WholeType { size, never_negative: false })),
            "decimal" => Some(Type::Decimal { size, fast: false }),
            "fraction" => Some(Type::Fraction(size)),
            _ => None,
        }
    }
}

/// Every type name, for "did you mean".
pub const TYPE_NAMES: &[&str] = &[
    "tiny_whole",
    "small_whole",
    "medium_whole",
    "big_whole",
    "tiny_decimal",
    "small_decimal",
    "medium_decimal",
    "big_decimal",
    "tiny_fraction",
    "small_fraction",
    "medium_fraction",
    "big_fraction",
    "text",
    "true_or_false",
];

/// Every modifier, for "did you mean".
pub const MODIFIER_NAMES: &[&str] = &["never_negative", "can't_change", "fast_but_inexact", "share"];

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let kept = row[j + 1];
            row[j + 1] = if ca == cb { previous } else { 1 + previous.min(row[j]).min(row[j + 1]) };
            previous = kept;
        }
    }
    row[b.len()]
}

// trace: decision.errors.layout
/// The one near match, when it is certain: the only name within one change of a short word, or two of a longer one.
pub fn near_match<'a>(written: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let allowed = if written.chars().count() <= 3 { 1 } else { 2 };
    let mut found = None;
    for candidate in candidates {
        if candidate != written && distance(written, candidate) <= allowed {
            if found.is_some_and(|before| before != candidate) {
                return None;
            }
            found = Some(candidate);
        }
    }
    found
}
