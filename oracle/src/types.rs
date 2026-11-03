//! TAKL's types, and the rules a variable carries with its type.

use crate::numbers::Size;
use num_bigint::BigInt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Whole {
        size: Size,
        never_negative: bool,
    },
    Exact(Size),
    Fast(Size),
    Fraction(Size),
    Text,
    TrueOrFalse,
    Table {
        key: Option<Box<Ty>>,
        item: Box<Ty>,
    },
    /// A number written in the program, or worked out from written numbers, whose type comes from where it is used.
    WrittenWhole,
    WrittenDecimal,
    WrittenFraction,
    /// A type that could not be worked out because of an earlier error; nothing more is said about it.
    Unknown,
}

impl Ty {
    pub fn is_unknown(&self) -> bool {
        match self {
            Ty::Unknown => true,
            Ty::Table { key, item } => item.is_unknown() || key.as_ref().is_some_and(|key| key.is_unknown()),
            _ => false,
        }
    }

    pub fn is_written(&self) -> bool {
        matches!(self, Ty::WrittenWhole | Ty::WrittenDecimal | Ty::WrittenFraction)
    }

    pub fn is_number(&self) -> bool {
        matches!(
            self,
            Ty::Whole { .. }
                | Ty::Exact(_)
                | Ty::Fast(_)
                | Ty::Fraction(_)
                | Ty::WrittenWhole
                | Ty::WrittenDecimal
                | Ty::WrittenFraction
        )
    }

    /// How the type is written, for messages.
    pub fn name(&self) -> String {
        match self {
            Ty::Whole { size, never_negative: false } => format!("{}_whole", size.word()),
            Ty::Whole { size, never_negative: true } => format!("{}_whole, never_negative", size.word()),
            Ty::Exact(size) => format!("{}_decimal", size.word()),
            Ty::Fast(size) => format!("{}_decimal, fast_but_inexact", size.word()),
            Ty::Fraction(size) => format!("{}_fraction", size.word()),
            Ty::Text => "text".to_string(),
            Ty::TrueOrFalse => "true_or_false".to_string(),
            Ty::Table { key: None, item } => format!("table of {}", item.item_name()),
            Ty::Table { key: Some(key), item } => format!("table of {} to {}", key.item_name(), item.item_name()),
            Ty::WrittenWhole => "whole number".to_string(),
            Ty::WrittenDecimal => "decimal".to_string(),
            Ty::WrittenFraction => "fraction".to_string(),
            Ty::Unknown => "?".to_string(),
        }
    }

    fn item_name(&self) -> String {
        match self {
            Ty::Whole { never_negative: true, .. } | Ty::Fast(_) => format!("({})", self.name()),
            _ => self.name(),
        }
    }
}

/// What a variable or param may do, beyond its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub ty: Ty,
    pub can_change: bool,
    /// trace: decision.types.bounds
    pub at_least: Option<BigInt>,
    pub at_most: Option<BigInt>,
    pub share: bool,
}

impl Rules {
    pub fn of(ty: Ty) -> Rules {
        Rules { ty, can_change: true, at_least: None, at_most: None, share: false }
    }
}

/// The names of the number types: `<size>_whole`, `<size>_decimal`, `<size>_fraction`.
pub fn number_type(name: &str) -> Option<Ty> {
    let (size, kind) = name.split_once('_')?;
    let size = Size::from_word(size)?;
    match kind {
        "whole" => Some(Ty::Whole { size, never_negative: false }),
        "decimal" => Some(Ty::Exact(size)),
        "fraction" => Some(Ty::Fraction(size)),
        _ => None,
    }
}

/// Every type name TAKL knows, for "did you mean".
pub fn type_names() -> Vec<String> {
    let mut names = vec!["text".to_string(), "true_or_false".to_string()];
    for size in ["tiny", "small", "medium", "big"] {
        for kind in ["whole", "decimal", "fraction"] {
            names.push(format!("{}_{}", size, kind));
        }
    }
    names
}
