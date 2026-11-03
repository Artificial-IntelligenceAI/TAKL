//! TAKL's numbers: whole numbers by size, exact decimals, fast decimals and fractions.
//!
//! trace: decision.types.whole
//! trace: decision.types.decimals
//!
//! These are the plain, always-right versions that compile time, the interpreter's slow paths and every error message
//! use. The interpreter and Cranelift run the common whole-number and float cases inline, and fall back here.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rustc_apfloat::Float;
use rustc_apfloat::ieee::{Double, Half, Quad, Single};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Size {
    Tiny,
    Small,
    Medium,
    Big,
}

impl Size {
    pub fn word(self) -> &'static str {
        match self {
            Size::Tiny => "tiny",
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Big => "big",
        }
    }

    pub fn bits(self) -> u32 {
        match self {
            Size::Tiny => 8,
            Size::Small => 16,
            Size::Medium => 32,
            Size::Big => 64,
        }
    }

    // trace: decision.types.decimals
    /// How many digits an exact decimal of this size holds.
    pub fn digits(self) -> u64 {
        match self {
            Size::Tiny => 7,
            Size::Small => 16,
            Size::Medium => 34,
            Size::Big => 70,
        }
    }

    pub fn from_word(word: &str) -> Option<Size> {
        Some(match word {
            "tiny" => Size::Tiny,
            "small" => Size::Small,
            "medium" => Size::Medium,
            "big" => Size::Big,
            _ => return None,
        })
    }
}

/// Why a piece of maths can't give an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    TooBig(BigInt),
    TooSmall(BigInt),
    DividesByZero,
    /// A whole-number division with something left over: the top and the bottom.
    Uneven(BigInt, BigInt),
    GoesOnForever,
    TooManyDigits,
    /// `round` exactly halfway, which isn't designed.
    Halfway,
    /// A fast decimal past the biggest it holds.
    FastTooBig,
    /// A remainder with a number below zero, which isn't designed.
    NegativeRemainder,
}

// trace: decision.types.never_negative
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WholeType {
    pub size: Size,
    pub never_negative: bool,
}

impl WholeType {
    pub fn smallest(self) -> i128 {
        if self.never_negative { 0 } else { -(1i128 << (self.size.bits() - 1)) }
    }

    pub fn biggest(self) -> i128 {
        if self.never_negative { (1i128 << self.size.bits()) - 1 } else { (1i128 << (self.size.bits() - 1)) - 1 }
    }

    pub fn name(self) -> String {
        if self.never_negative {
            format!("{}_whole, never_negative", self.size.word())
        } else {
            format!("{}_whole", self.size.word())
        }
    }

    pub fn holds(self) -> String {
        format!("{} to {}", self.smallest(), self.biggest())
    }

    /// What other languages call it, with its article.
    pub fn other_name(self) -> String {
        if self.never_negative { format!("a uint{}", self.size.bits()) } else { format!("an int{}", self.size.bits()) }
    }

    // trace: decision.numbers.range
    pub fn check(self, value: i128) -> Result<i128, Problem> {
        if value > self.biggest() {
            Err(Problem::TooBig(BigInt::from(value)))
        } else if value < self.smallest() {
            Err(Problem::TooSmall(BigInt::from(value)))
        } else {
            Ok(value)
        }
    }

    pub fn check_big(self, value: &BigInt) -> Result<i128, Problem> {
        match value.to_i128() {
            Some(small) => self.check(small),
            None if value.is_negative() => Err(Problem::TooSmall(value.clone())),
            None => Err(Problem::TooBig(value.clone())),
        }
    }
}

/// An exact decimal: `coefficient` times ten to the power of minus `scale`. `2.50` is 250 with scale 2, so it shows
/// as written; a scale below zero is a whole number with zeros on the end.
// trace: decision.types.exact_bare
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    pub coefficient: BigInt,
    pub scale: i64,
}

fn ten_to(power: u64) -> BigInt {
    num_traits::pow(BigInt::from(10u8), power as usize)
}

fn digit_count(value: &BigInt) -> u64 {
    if value.is_zero() { 1 } else { value.abs().to_string().len() as u64 }
}

impl Decimal {
    pub fn written(whole: &str, after_point: Option<&str>) -> Decimal {
        let digits = format!("{}{}", whole, after_point.unwrap_or(""));
        Decimal { coefficient: digits.parse().unwrap_or_default(), scale: after_point.map_or(0, |d| d.len() as i64) }
    }

    pub fn from_whole(value: BigInt) -> Decimal {
        Decimal { coefficient: value, scale: 0 }
    }

    pub fn is_zero(&self) -> bool {
        self.coefficient.is_zero()
    }

    /// As a fraction, top over bottom, the bottom above zero.
    pub fn rational(&self) -> (BigInt, BigInt) {
        if self.scale >= 0 {
            (self.coefficient.clone(), ten_to(self.scale as u64))
        } else {
            (&self.coefficient * ten_to((-self.scale) as u64), BigInt::one())
        }
    }

    // trace: decision.decimals.rounding
    /// Drops zeros on the end only while the number has more digits than it may hold; an exact decimal never rounds.
    pub fn fit(mut self, digits: Option<u64>) -> Result<Decimal, Problem> {
        let Some(limit) = digits else { return Ok(self) };
        while digit_count(&self.coefficient) > limit {
            let (quotient, rest) = self.coefficient.div_rem(&BigInt::from(10u8));
            if !rest.is_zero() {
                return Err(Problem::TooManyDigits);
            }
            self.coefficient = quotient;
            self.scale -= 1;
        }
        Ok(self)
    }

    fn aligned(&self, other: &Decimal) -> (BigInt, BigInt, i64) {
        let scale = self.scale.max(other.scale);
        let left = &self.coefficient * ten_to((scale - self.scale) as u64);
        let right = &other.coefficient * ten_to((scale - other.scale) as u64);
        (left, right, scale)
    }

    pub fn plus(&self, other: &Decimal, digits: Option<u64>) -> Result<Decimal, Problem> {
        let (left, right, scale) = self.aligned(other);
        Decimal { coefficient: left + right, scale }.fit(digits)
    }

    pub fn minus(&self, other: &Decimal, digits: Option<u64>) -> Result<Decimal, Problem> {
        let (left, right, scale) = self.aligned(other);
        Decimal { coefficient: left - right, scale }.fit(digits)
    }

    pub fn times(&self, other: &Decimal, digits: Option<u64>) -> Result<Decimal, Problem> {
        Decimal { coefficient: &self.coefficient * &other.coefficient, scale: self.scale + other.scale }.fit(digits)
    }

    /// The exact answer, at the scale the two sides suggest or the smallest that holds it.
    pub fn divided_by(&self, other: &Decimal, digits: Option<u64>) -> Result<Decimal, Problem> {
        if other.is_zero() {
            return Err(Problem::DividesByZero);
        }
        let (top, bottom) = self.quotient(other);
        let mut scale = self.scale - other.scale;
        loop {
            let scaled = if scale >= 0 { &top * ten_to(scale as u64) } else { top.clone() };
            let bottom_scaled = if scale >= 0 { bottom.clone() } else { &bottom * ten_to((-scale) as u64) };
            let (quotient, rest) = scaled.div_rem(&bottom_scaled);
            if rest.is_zero() {
                return Decimal { coefficient: quotient, scale }.fit(digits);
            }
            if !only_twos_and_fives(&bottom) {
                return Err(Problem::GoesOnForever);
            }
            scale += 1;
        }
    }

    /// `self / other` as a fraction in lowest terms, the bottom above zero.
    fn quotient(&self, other: &Decimal) -> (BigInt, BigInt) {
        let (a_top, a_bottom) = self.rational();
        let (b_top, b_bottom) = other.rational();
        let mut top = a_top * b_bottom;
        let mut bottom = a_bottom * b_top;
        if bottom.is_negative() {
            top = -top;
            bottom = -bottom;
        }
        let common = top.gcd(&bottom);
        if !common.is_zero() && !common.is_one() {
            top /= &common;
            bottom /= &common;
        }
        (top, bottom)
    }

    // trace: decision.decimals.rounding
    /// `round (a divided by b)`: the exact answer if it fits, else the nearest with as many digits as the type holds.
    pub fn divided_by_rounded(&self, other: &Decimal, digits: Option<u64>) -> Result<Decimal, Problem> {
        match self.divided_by(other, digits) {
            Ok(answer) => return Ok(answer),
            Err(Problem::GoesOnForever | Problem::TooManyDigits) if digits.is_some() => {}
            Err(problem) => return Err(problem),
        }
        let limit = digits.unwrap_or(0) as i64;
        let (top, bottom) = self.quotient(other);
        // Find the scale that leaves `limit` digits before rounding: the answer's first digit is at 10^lead.
        let magnitude_top = top.abs();
        let mut lead = digit_count(&magnitude_top) as i64 - digit_count(&bottom) as i64;
        if compare_scaled(&magnitude_top, &bottom, lead) == Ordering::Less {
            lead -= 1;
        }
        let scale = limit - 1 - lead;
        let (scaled_top, scaled_bottom) = if scale >= 0 {
            (top * ten_to(scale as u64), bottom)
        } else {
            (top, bottom * ten_to((-scale) as u64))
        };
        let rounded = round_nearest(&scaled_top, &scaled_bottom)?;
        Decimal { coefficient: rounded, scale }.fit(digits)
    }

    pub fn compare(&self, other: &Decimal) -> Ordering {
        let (left, right, _) = self.aligned(other);
        left.cmp(&right)
    }

    /// As it is held: `2.50` shows as `2.50`.
    // trace: decision.show.decimals
    pub fn show(&self) -> String {
        let negative = self.coefficient.is_negative();
        let digits = self.coefficient.abs().to_string();
        let body = if self.scale <= 0 {
            if self.coefficient.is_zero() { "0".to_string() } else { format!("{}{}", digits, "0".repeat((-self.scale) as usize)) }
        } else {
            let scale = self.scale as usize;
            let padded = if digits.len() <= scale { format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits) } else { digits };
            let (whole, after) = padded.split_at(padded.len() - scale);
            format!("{}.{}", whole, after)
        };
        if negative { format!("-{}", body) } else { body }
    }
}

/// `top * 10^power` against `bottom`.
fn compare_scaled(top: &BigInt, bottom: &BigInt, power: i64) -> Ordering {
    if power >= 0 { (top.clone()).cmp(&(bottom * ten_to(power as u64))) } else { (top * ten_to((-power) as u64)).cmp(bottom) }
}

fn only_twos_and_fives(value: &BigInt) -> bool {
    let mut left = value.abs();
    for prime in [2u8, 5u8] {
        let prime = BigInt::from(prime);
        while !left.is_zero() && (&left % &prime).is_zero() {
            left /= &prime;
        }
    }
    left.is_one()
}

/// `top / bottom` to the nearest whole number; exactly halfway isn't designed. The bottom is above zero.
pub fn round_nearest(top: &BigInt, bottom: &BigInt) -> Result<BigInt, Problem> {
    let (quotient, rest) = top.div_mod_floor(bottom);
    let twice = &rest * 2u8;
    match twice.cmp(bottom) {
        Ordering::Less => Ok(quotient),
        Ordering::Greater => Ok(quotient + 1u8),
        Ordering::Equal => Err(Problem::Halfway),
    }
}

/// Whole-number division, which must come out even.
pub fn whole_divided_by(top: &BigInt, bottom: &BigInt) -> Result<BigInt, Problem> {
    if bottom.is_zero() {
        return Err(Problem::DividesByZero);
    }
    let (quotient, rest) = top.div_rem(bottom);
    if rest.is_zero() { Ok(quotient) } else { Err(Problem::Uneven(top.clone(), bottom.clone())) }
}

// trace: decision.maths.remainder
pub fn whole_remainder(top: &BigInt, bottom: &BigInt) -> Result<BigInt, Problem> {
    if bottom.is_zero() {
        return Err(Problem::DividesByZero);
    }
    if top.is_negative() || bottom.is_negative() {
        return Err(Problem::NegativeRemainder);
    }
    Ok(top % bottom)
}

pub fn whole_rounded(top: &BigInt, bottom: &BigInt) -> Result<BigInt, Problem> {
    if bottom.is_zero() {
        return Err(Problem::DividesByZero);
    }
    if bottom.is_negative() { round_nearest(&-top, &-bottom) } else { round_nearest(top, bottom) }
}

// trace: decision.types.approx
// trace: decision.types.fast_but_inexact
/// Fast decimals, held as IEEE binary floats in the low bits of a u128: tiny is a float16, small a float32, medium a
/// float64, big a float128.
pub mod fast {
    use super::*;

    fn layout(size: Size) -> (u32, u32) {
        match size {
            Size::Tiny => (5, 10),
            Size::Small => (8, 23),
            Size::Medium => (11, 52),
            Size::Big => (15, 112),
        }
    }

    pub enum Written {
        Exact(u128),
        /// The nearest it holds, which isn't what was written.
        Nearest(u128),
        TooBig,
    }

    fn parse<F: Float>(text: &str) -> Written {
        match F::from_str_r(text, rustc_apfloat::Round::NearestTiesToEven) {
            Ok(result) => {
                if result.value.is_infinite() {
                    Written::TooBig
                } else if result.status.is_empty() {
                    Written::Exact(result.value.to_bits())
                } else {
                    Written::Nearest(result.value.to_bits())
                }
            }
            Err(_) => Written::TooBig,
        }
    }

    /// A written number, as a fast decimal of this size holds it.
    pub fn written(size: Size, text: &str) -> Written {
        match size {
            Size::Tiny => parse::<Half>(text),
            Size::Small => parse::<Single>(text),
            Size::Medium => parse::<Double>(text),
            Size::Big => parse::<Quad>(text),
        }
    }

    pub fn negative(size: Size, bits: u128) -> bool {
        let (exponent, fraction) = layout(size);
        (bits >> (exponent + fraction)) & 1 == 1
    }

    // trace: decision.show.decimals
    /// Every digit of the value it holds, as `show` prints it.
    pub fn show(size: Size, bits: u128) -> String {
        let (exponent_bits, fraction_bits) = layout(size);
        let sign = negative(size, bits);
        let exponent = ((bits >> fraction_bits) & ((1u128 << exponent_bits) - 1)) as i64;
        let fraction = bits & ((1u128 << fraction_bits) - 1);
        let bias = (1i64 << (exponent_bits - 1)) - 1;
        let (mantissa, power) = if exponent == 0 {
            (fraction, 1 - bias - fraction_bits as i64)
        } else {
            (fraction | (1u128 << fraction_bits), exponent - bias - fraction_bits as i64)
        };
        let body = if mantissa == 0 {
            "0".to_string()
        } else if power >= 0 {
            (BigInt::from(mantissa) << power as usize).to_string()
        } else {
            let places = (-power) as u64;
            let scaled = BigInt::from(mantissa) * num_traits::pow(BigInt::from(5u8), places as usize);
            Decimal { coefficient: scaled, scale: places as i64 }.show_trimmed()
        };
        if sign && mantissa != 0 { format!("-{}", body) } else { body }
    }

    /// For "the nearest it holds is ...": the first 36 characters, then `…` if there were more.
    pub fn nearest_text(size: Size, bits: u128) -> String {
        let full = show(size, bits);
        if full.chars().count() > 36 { format!("{}…", full.chars().take(36).collect::<String>()) } else { full }
    }

    fn quad(bits: u128) -> Quad {
        Quad::from_bits(bits)
    }

    fn checked(size: Size, bits: u128) -> Result<u128, Problem> {
        let (exponent_bits, fraction_bits) = layout(size);
        let exponent = (bits >> fraction_bits) & ((1u128 << exponent_bits) - 1);
        if exponent == (1u128 << exponent_bits) - 1 { Err(Problem::FastTooBig) } else { Ok(bits) }
    }

    pub fn is_zero(size: Size, bits: u128) -> bool {
        let (exponent_bits, fraction_bits) = layout(size);
        bits & ((1u128 << (exponent_bits + fraction_bits)) - 1) == 0
    }

    pub fn maths(size: Size, operator: crate::syntax::Maths, a: u128, b: u128) -> Result<u128, Problem> {
        use crate::syntax::Maths;
        if operator == Maths::DividedBy && is_zero(size, b) {
            return Err(Problem::DividesByZero);
        }
        let bits = match size {
            Size::Tiny => {
                // Each of + - * / on two float16s done in float32 and rounded once more is correctly rounded,
                // since 24 bits is at least twice 11 plus 2.
                let x = half::f16::from_bits(a as u16).to_f32();
                let y = half::f16::from_bits(b as u16).to_f32();
                let answer = match operator {
                    Maths::Plus => x + y,
                    Maths::Minus => x - y,
                    Maths::Times => x * y,
                    Maths::DividedBy => x / y,
                };
                half::f16::from_f32(answer).to_bits() as u128
            }
            Size::Small => {
                let x = f32::from_bits(a as u32);
                let y = f32::from_bits(b as u32);
                (match operator {
                    Maths::Plus => x + y,
                    Maths::Minus => x - y,
                    Maths::Times => x * y,
                    Maths::DividedBy => x / y,
                })
                .to_bits() as u128
            }
            Size::Medium => {
                let x = f64::from_bits(a as u64);
                let y = f64::from_bits(b as u64);
                (match operator {
                    Maths::Plus => x + y,
                    Maths::Minus => x - y,
                    Maths::Times => x * y,
                    Maths::DividedBy => x / y,
                })
                .to_bits() as u128
            }
            Size::Big => {
                let (x, y) = (quad(a), quad(b));
                let answer = match operator {
                    Maths::Plus => x + y,
                    Maths::Minus => x - y,
                    Maths::Times => x * y,
                    Maths::DividedBy => x / y,
                };
                answer.value.to_bits()
            }
        };
        checked(size, bits)
    }

    pub fn compare(size: Size, a: u128, b: u128) -> Ordering {
        let ordering = match size {
            Size::Tiny => half::f16::from_bits(a as u16).partial_cmp(&half::f16::from_bits(b as u16)),
            Size::Small => f32::from_bits(a as u32).partial_cmp(&f32::from_bits(b as u32)),
            Size::Medium => f64::from_bits(a as u64).partial_cmp(&f64::from_bits(b as u64)),
            Size::Big => quad(a).partial_cmp(&quad(b)),
        };
        ordering.unwrap_or(Ordering::Equal)
    }

    /// A whole number as a fast decimal, for `average of`'s count.
    pub fn from_whole(size: Size, value: i128) -> u128 {
        match written(size, &value.to_string()) {
            Written::Exact(bits) | Written::Nearest(bits) => bits,
            Written::TooBig => 0,
        }
    }
}

impl Decimal {
    /// Like `show`, with the zeros after the point that a binary float's expansion leaves taken off.
    fn show_trimmed(&self) -> String {
        let shown = self.show();
        if shown.contains('.') { shown.trim_end_matches('0').trim_end_matches('.').to_string() } else { shown }
    }
}
