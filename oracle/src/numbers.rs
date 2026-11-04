//! Whole numbers, exact decimals, fast decimals and fractions, each held exactly as its type says.

use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rustc_apfloat::ieee::{Double, Half, Quad, Single};
use rustc_apfloat::{Float, Round, Status};
use std::cmp::Ordering;

/// The four sizes every number type comes in.
/// trace: decision.types.whole
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

    pub fn from_word(word: &str) -> Option<Size> {
        match word {
            "tiny" => Some(Size::Tiny),
            "small" => Some(Size::Small),
            "medium" => Some(Size::Medium),
            "big" => Some(Size::Big),
            _ => None,
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
}

// ---- whole numbers ----------------------------------------------------------------------------------------------

/// The smallest and biggest a whole number of this size holds.
/// trace: decision.types.never_negative (unsigned, so the top doubles)
pub fn whole_range(size: Size, never_negative: bool) -> (BigInt, BigInt) {
    let bits = size.bits();
    if never_negative {
        (BigInt::zero(), (BigInt::one() << bits) - 1)
    } else {
        (-(BigInt::one() << (bits - 1)), (BigInt::one() << (bits - 1)) - 1)
    }
}

pub fn whole_fits(n: &BigInt, size: Size, never_negative: bool) -> bool {
    let (least, most) = whole_range(size, never_negative);
    *n >= least && *n <= most
}

// ---- exact decimals ---------------------------------------------------------------------------------------------

/// How many digits an exact decimal holds and how far its point may move: IEEE 754's decimal32, decimal64 and
/// decimal128, and decimal256 from the standard's formula for wider formats (70 digits, biggest power 1,572,864).
/// trace: decision.types.decimals
pub fn decimal_digits(size: Size) -> u32 {
    match size {
        Size::Tiny => 7,
        Size::Small => 16,
        Size::Medium => 34,
        Size::Big => 70,
    }
}

fn decimal_emax(size: Size) -> i64 {
    match size {
        Size::Tiny => 96,
        Size::Small => 384,
        Size::Medium => 6144,
        Size::Big => 1_572_864,
    }
}

/// A decimal as held: a whole number of units and where the point sits, so `2.50` stays `2.50`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exact {
    pub coefficient: BigInt,
    pub exponent: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The answer's digits never end, as `1 divided by 3`.
    GoesOnForever,
    /// The answer has more digits than the type holds.
    TooManyDigits,
    TooBig,
    TooSmall,
    DividedByZero,
    /// Rounding landed exactly halfway, and which way to go is not designed.
    Halfway,
}

fn digit_count(n: &BigInt) -> u32 {
    if n.is_zero() { 1 } else { n.abs().to_string().len() as u32 }
}

fn ten_to(power: u64) -> BigInt {
    num_traits::pow(BigInt::from(10), power as usize)
}

impl Exact {
    pub fn whole(n: BigInt) -> Exact {
        Exact { coefficient: n, exponent: 0 }
    }

    /// Keeps the number in this size, moving the point only as far as needed and never rounding.
    /// trace: decision.decimals.rounding (an exact decimal never rounds on its own)
    pub fn fit(self, size: Option<Size>) -> Result<Exact, Trouble> {
        let Some(size) = size else { return Ok(self) };
        let digits = decimal_digits(size);
        let emax = decimal_emax(size);
        let low = 1 - emax - (digits as i64 - 1);
        let high = emax - (digits as i64 - 1);
        let mut coefficient = self.coefficient;
        let mut exponent = self.exponent;
        if coefficient.is_zero() {
            return Ok(Exact { coefficient, exponent: exponent.clamp(low, high) });
        }
        // Zeros on the end can move into the exponent, or back out of it, without changing the number.
        let count = digit_count(&coefficient) as i64;
        let zeros = trailing_zeros(&coefficient) as i64;
        let over = count - digits as i64;
        let strip = over.max(low - exponent).max(0).min(zeros);
        if strip > 0 {
            coefficient /= ten_to(strip as u64);
            exponent += strip;
        }
        if digit_count(&coefficient) > digits {
            return Err(Trouble::TooManyDigits);
        }
        if exponent < low {
            return Err(Trouble::TooSmall);
        }
        if exponent > high {
            let room = digits as i64 - digit_count(&coefficient) as i64;
            let add = room.min(exponent - high);
            coefficient *= ten_to(add as u64);
            exponent -= add;
        }
        if exponent > high {
            return Err(Trouble::TooBig);
        }
        Ok(Exact { coefficient, exponent })
    }

    fn aligned(&self, other: &Exact) -> (BigInt, BigInt, i64) {
        let exponent = self.exponent.min(other.exponent);
        let a = &self.coefficient * ten_to((self.exponent - exponent) as u64);
        let b = &other.coefficient * ten_to((other.exponent - exponent) as u64);
        (a, b, exponent)
    }

    pub fn plus(&self, other: &Exact, size: Option<Size>) -> Result<Exact, Trouble> {
        let (a, b, exponent) = self.aligned(other);
        Exact { coefficient: a + b, exponent }.fit(size)
    }

    pub fn minus(&self, other: &Exact, size: Option<Size>) -> Result<Exact, Trouble> {
        let (a, b, exponent) = self.aligned(other);
        Exact { coefficient: a - b, exponent }.fit(size)
    }

    pub fn times(&self, other: &Exact, size: Option<Size>) -> Result<Exact, Trouble> {
        Exact { coefficient: &self.coefficient * &other.coefficient, exponent: self.exponent + other.exponent }
            .fit(size)
    }

    /// The exact answer, with the point as near as it can be to where the two numbers' points put it.
    pub fn divided_by(&self, other: &Exact, size: Option<Size>) -> Result<Exact, Trouble> {
        if other.coefficient.is_zero() {
            return Err(Trouble::DividedByZero);
        }
        let ideal = self.exponent - other.exponent;
        let (top, bottom) = reduce(self.coefficient.clone(), other.coefficient.clone());
        let Some((coefficient, exponent)) = terminating(&top, &bottom) else {
            return Err(Trouble::GoesOnForever);
        };
        let exponent = exponent + ideal;
        let zeros = trailing_zeros(&coefficient);
        let chosen = ideal.min(exponent + zeros as i64);
        let coefficient = if chosen >= exponent {
            coefficient / ten_to((chosen - exponent) as u64)
        } else {
            coefficient * ten_to((exponent - chosen) as u64)
        };
        Exact { coefficient, exponent: chosen }.fit(size)
    }

    /// Rounds to the nearest this size holds, on purpose; only what doesn't fit is rounded.
    /// trace: decision.decimals.rounding (`round (1 / 3)` rounds on purpose)
    pub fn round_division(&self, other: &Exact, size: Option<Size>) -> Result<Exact, Trouble> {
        match self.divided_by(other, size) {
            Err(Trouble::GoesOnForever | Trouble::TooManyDigits | Trouble::TooSmall) => {}
            done => return done,
        }
        let Some(size) = size else { return Err(Trouble::GoesOnForever) };
        let digits = decimal_digits(size) as i64;
        let low = 1 - decimal_emax(size) - (digits - 1);
        let (top, bottom) = self.ratio(other);
        // The power of ten at or just below the answer, so the rounded answer keeps every digit the size holds.
        let mut magnitude = top.abs().to_string().len() as i64 - bottom.to_string().len() as i64;
        while scaled_compare(&top.abs(), &bottom, magnitude) == Ordering::Less {
            magnitude -= 1;
        }
        while scaled_compare(&top.abs(), &bottom, magnitude + 1) != Ordering::Less {
            magnitude += 1;
        }
        let exponent = (magnitude - digits + 1).max(low);
        let (scaled_top, scaled_bottom) = if exponent < 0 {
            (&top * ten_to((-exponent) as u64), bottom.clone())
        } else {
            (top.clone(), &bottom * ten_to(exponent as u64))
        };
        let coefficient = round_nearest(&scaled_top, &scaled_bottom)?;
        Exact { coefficient, exponent }.fit(Some(size))
    }

    /// The exact value of `self / other` as a reduced fraction with a positive bottom.
    fn ratio(&self, other: &Exact) -> (BigInt, BigInt) {
        let shift = self.exponent - other.exponent;
        let (top, bottom) = if shift >= 0 {
            (&self.coefficient * ten_to(shift as u64), other.coefficient.clone())
        } else {
            (self.coefficient.clone(), &other.coefficient * ten_to((-shift) as u64))
        };
        reduce(top, bottom)
    }

    pub fn compare(&self, other: &Exact) -> Ordering {
        let (a, b, _) = self.aligned(other);
        a.cmp(&b)
    }

    /// trace: decision.show.decimals (as it is held: `2.50` prints `2.50`)
    pub fn to_text(&self) -> String {
        let negative = self.coefficient.is_negative();
        let digits = self.coefficient.abs().to_string();
        let body = if self.coefficient.is_zero() && self.exponent >= 0 {
            // Zero is one digit, however far its point has moved.
            digits
        } else if self.exponent >= 0 {
            format!("{}{}", digits, "0".repeat(self.exponent as usize))
        } else {
            let places = (-self.exponent) as usize;
            let padded = if digits.len() <= places {
                format!("{}{}", "0".repeat(places - digits.len() + 1), digits)
            } else {
                digits
            };
            let point = padded.len() - places;
            format!("{}.{}", &padded[..point], &padded[point..])
        };
        if negative { format!("-{}", body) } else { body }
    }
}

/// Compares `a / b` with `10^power`.
fn scaled_compare(a: &BigInt, b: &BigInt, power: i64) -> Ordering {
    if power >= 0 { a.cmp(&(b * ten_to(power as u64))) } else { (a * ten_to((-power) as u64)).cmp(b) }
}

fn trailing_zeros(n: &BigInt) -> u32 {
    if n.is_zero() {
        return 0;
    }
    n.to_string().bytes().rev().take_while(|digit| *digit == b'0').count() as u32
}

fn reduce(top: BigInt, bottom: BigInt) -> (BigInt, BigInt) {
    let divisor = top.gcd(&bottom);
    let (mut top, mut bottom) = if divisor.is_zero() { (top, bottom) } else { (top / &divisor, bottom / &divisor) };
    if bottom.is_negative() {
        top = -top;
        bottom = -bottom;
    }
    (top, bottom)
}

/// `top / bottom` as `coefficient × 10^exponent` when its digits end, with the exponent as large as can be.
fn terminating(top: &BigInt, bottom: &BigInt) -> Option<(BigInt, i64)> {
    let mut rest = bottom.clone();
    let mut twos = 0u64;
    let mut fives = 0u64;
    let two = BigInt::from(2);
    let five = BigInt::from(5);
    while (&rest % &two).is_zero() {
        rest /= &two;
        twos += 1;
    }
    while (&rest % &five).is_zero() {
        rest /= &five;
        fives += 1;
    }
    if !rest.is_one() {
        return None;
    }
    let places = twos.max(fives);
    let coefficient =
        top * num_traits::pow(two, (places - twos) as usize) * num_traits::pow(five, (places - fives) as usize);
    Some((coefficient, -(places as i64)))
}

/// The nearest whole number to `top / bottom`; exactly halfway is not designed.
pub fn round_nearest(top: &BigInt, bottom: &BigInt) -> Result<BigInt, Trouble> {
    let (top, bottom) = reduce(top.clone(), bottom.clone());
    let (quotient, remainder) = top.div_mod_floor(&bottom);
    let twice: BigInt = &remainder * 2;
    match twice.cmp(&bottom) {
        Ordering::Less => Ok(quotient),
        Ordering::Greater => Ok(quotient + 1),
        Ordering::Equal => Err(Trouble::Halfway),
    }
}

// ---- fast decimals ----------------------------------------------------------------------------------------------

/// A fast decimal is an IEEE binary float, worked in software so every machine gives the same bits.
/// trace: decision.types.fast_but_inexact
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fast {
    pub size: Size,
    pub bits: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FastOp {
    Plus,
    Minus,
    Times,
    DividedBy,
}

fn fast_parse<F: Float>(text: &str) -> Result<(u128, bool), Trouble> {
    let parsed = F::from_str_r(text, Round::NearestTiesToEven).map_err(|_| Trouble::TooBig)?;
    if parsed.status.contains(Status::OVERFLOW) || !parsed.value.is_finite() {
        return Err(Trouble::TooBig);
    }
    Ok((parsed.value.to_bits(), parsed.status == Status::OK))
}

fn fast_apply<F: Float>(a: u128, b: u128, op: FastOp) -> Result<u128, Trouble> {
    let (a, b) = (F::from_bits(a), F::from_bits(b));
    if op == FastOp::DividedBy && b.is_zero() {
        return Err(Trouble::DividedByZero);
    }
    let result = match op {
        FastOp::Plus => a.add_r(b, Round::NearestTiesToEven),
        FastOp::Minus => a.sub_r(b, Round::NearestTiesToEven),
        FastOp::Times => a.mul_r(b, Round::NearestTiesToEven),
        FastOp::DividedBy => a.div_r(b, Round::NearestTiesToEven),
    };
    if result.status.contains(Status::OVERFLOW) || !result.value.is_finite() {
        return Err(Trouble::TooBig);
    }
    Ok(result.value.to_bits())
}

fn fast_compare<F: Float>(a: u128, b: u128) -> Ordering {
    F::from_bits(a).partial_cmp(&F::from_bits(b)).unwrap_or(Ordering::Equal)
}

/// The exact value a float's bits hold: sign, a whole number, and a power of two.
fn fast_parts<F: Float>(bits: u128) -> (bool, BigUint, i64) {
    let total = F::BITS as u32;
    let mantissa_bits = (F::PRECISION - 1) as u32;
    let exponent_bits = total - 1 - mantissa_bits;
    let negative = (bits >> (total - 1)) & 1 == 1;
    let field = ((bits >> mantissa_bits) & ((1u128 << exponent_bits) - 1)) as i64;
    let mantissa = bits & ((1u128 << mantissa_bits) - 1);
    let bias = (1i64 << (exponent_bits - 1)) - 1;
    if field == 0 {
        (negative, BigUint::from(mantissa), 1 - bias - mantissa_bits as i64)
    } else {
        (negative, BigUint::from(mantissa | (1u128 << mantissa_bits)), field - bias - mantissa_bits as i64)
    }
}

macro_rules! by_size {
    ($size:expr, $f:ident, $($arg:expr),*) => {
        match $size {
            Size::Tiny => $f::<Half>($($arg),*),
            Size::Small => $f::<Single>($($arg),*),
            Size::Medium => $f::<Double>($($arg),*),
            Size::Big => $f::<Quad>($($arg),*),
        }
    };
}

impl Fast {
    /// A written number, and whether the float holds it exactly.
    pub fn parse(text: &str, size: Size) -> Result<(Fast, bool), Trouble> {
        let (bits, exact) = by_size!(size, fast_parse, text)?;
        Ok((Fast { size, bits }, exact))
    }

    pub fn apply(&self, other: &Fast, op: FastOp) -> Result<Fast, Trouble> {
        let bits = by_size!(self.size, fast_apply, self.bits, other.bits, op)?;
        Ok(Fast { size: self.size, bits })
    }

    pub fn compare(&self, other: &Fast) -> Ordering {
        by_size!(self.size, fast_compare, self.bits, other.bits)
    }

    /// Every digit of the value the float holds; its digits always end, since it is a power of two apart.
    /// trace: decision.show.decimals (a fast decimal prints its whole held value)
    pub fn to_text(&self) -> String {
        let (negative, mut mantissa, mut exponent) = by_size!(self.size, fast_parts, self.bits);
        let body = if mantissa.is_zero() {
            "0".to_string()
        } else if exponent >= 0 {
            (mantissa << exponent as usize).to_string()
        } else {
            let two = BigUint::from(2u32);
            while exponent < 0 && (&mantissa % &two).is_zero() {
                mantissa /= &two;
                exponent += 1;
            }
            if exponent >= 0 {
                mantissa.to_string()
            } else {
                let places = (-exponent) as usize;
                let digits = (mantissa * num_traits::pow(BigUint::from(5u32), places)).to_string();
                let padded = if digits.len() <= places {
                    format!("{}{}", "0".repeat(places - digits.len() + 1), digits)
                } else {
                    digits
                };
                let point = padded.len() - places;
                format!("{}.{}", &padded[..point], &padded[point..])
            }
        };
        if negative { format!("-{}", body) } else { body }
    }

    /// The first digits of the nearest a written number the float holds, ending in `…`.
    pub fn nearest_shown(text: &str, size: Size) -> String {
        match Fast::parse(text, size) {
            Ok((fast, _)) => {
                let full = fast.to_text();
                if full.len() > 36 { format!("{}…", &full[..36]) } else { full }
            }
            Err(_) => text.to_string(),
        }
    }
}

// ---- fractions --------------------------------------------------------------------------------------------------

/// trace: decision.types.over
pub fn fraction_fits(top: &BigInt, bottom: &BigInt, size: Size) -> bool {
    whole_fits(top, size, false) && whole_fits(bottom, size, false)
}

pub fn in_lowest_terms(top: &BigInt, bottom: &BigInt) -> bool {
    top.gcd(bottom).is_one() && bottom.sign() == Sign::Plus
}

pub fn to_u8(n: &BigInt) -> Option<u8> {
    n.to_u8()
}
