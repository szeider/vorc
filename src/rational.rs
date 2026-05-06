//! Exact rational arithmetic utilities.
//!
//! Wraps num-rational with helpers for:
//! - Float-to-rational conversion with bounded denominator
//! - Exact ceiling/floor for integrality rounding
//! - Gaussian elimination for exact basis dual computation

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};

/// Project-wide exact rational type. Uses arbitrary-precision integers
/// to avoid overflow during Gaussian elimination and large-denominator cascades.
pub type Rat = BigRational;

/// Construct a Rat from numerator and denominator (test helper).
#[cfg(test)]
pub fn rat(numer: i64, denom: i64) -> Rat {
    Rat::new(BigInt::from(numer), BigInt::from(denom))
}

/// Construct a Rat from an integer.
pub fn rat_from_int(n: i64) -> Rat {
    Rat::from_integer(BigInt::from(n))
}

/// Convert a float to a rational with bounded denominator.
pub fn rat_from_float(x: f64, max_denom: i64) -> Rat {
    approximate_rational(x, max_denom)
}

/// Exact ceiling of a rational.
pub fn ceil(v: &Rat) -> Rat {
    if v.is_integer() {
        v.clone()
    } else if v.is_positive() {
        Rat::from_integer(v.numer() / v.denom() + BigInt::one())
    } else {
        Rat::from_integer(v.numer() / v.denom())
    }
}

/// Exact floor of a rational.
pub fn floor(v: &Rat) -> Rat {
    if v.is_integer() {
        v.clone()
    } else if v.is_negative() {
        Rat::from_integer(v.numer() / v.denom() - BigInt::one())
    } else {
        Rat::from_integer(v.numer() / v.denom())
    }
}

/// Round a rational to the nearest integer.
pub fn round(v: &Rat) -> Rat {
    if v.is_integer() {
        return v.clone();
    }
    // floor(v + 1/2)
    let half = Rat::new(BigInt::one(), BigInt::from(2));
    floor(&(v + half))
}

/// Convert a Rat to f64 (for passing to Gurobi).
pub fn rat_to_f64(r: &Rat) -> f64 {
    use num_traits::ToPrimitive;
    r.numer().to_f64().unwrap_or(f64::INFINITY)
        / r.denom().to_f64().unwrap_or(1.0)
}

/// Continued-fraction approximation of a float with bounded denominator.
/// Uses i128 intermediates to avoid overflow with large max_denom values (up to 10^18).
fn approximate_rational(x: f64, max_denom: i64) -> Rat {
    assert!(x.is_finite(), "approximate_rational: input must be finite, got {x}");
    if x == 0.0 {
        return Rat::zero();
    }
    let sign: i128 = if x < 0.0 { -1 } else { 1 };
    let x = x.abs();
    let max_denom = max_denom as i128;

    let mut p0: i128 = 0;
    let mut q0: i128 = 1;
    let mut p1: i128 = 1;
    let mut q1: i128 = 0;
    let mut val = x;

    loop {
        let a = val as i128;
        let p2 = match a.checked_mul(p1).and_then(|v| v.checked_add(p0)) {
            Some(v) => v,
            None => break, // overflow in numerator — stop with current best
        };
        let q2 = match a.checked_mul(q1).and_then(|v| v.checked_add(q0)) {
            Some(v) => v,
            None => break,
        };

        if q2 > max_denom {
            break;
        }

        p0 = p1;
        q0 = q1;
        p1 = p2;
        q1 = q2;

        let frac = val - a as f64;
        if frac < 1e-15 {
            break;
        }
        val = 1.0 / frac;

        if val > 1e15 {
            break;
        }
    }

    Rat::new(BigInt::from(sign * p1), BigInt::from(q1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ceil_floor() {
        let v = rat(7, 3); // 2.333...
        assert_eq!(ceil(&v), rat_from_int(3));
        assert_eq!(floor(&v), rat_from_int(2));

        let v = rat_from_int(5);
        assert_eq!(ceil(&v), rat_from_int(5));
        assert_eq!(floor(&v), rat_from_int(5));

        let v = rat(-7, 3); // -2.333...
        assert_eq!(ceil(&v), rat_from_int(-2));
        assert_eq!(floor(&v), rat_from_int(-3));
    }

    #[test]
    fn test_rat_from_float() {
        let r = rat_from_float(0.5, 1_000_000);
        assert_eq!(r, rat(1, 2));

        let r = rat_from_float(0.333333333, 1_000_000);
        assert_eq!(r, rat(1, 3));
    }

    #[test]
    fn test_large_denom() {
        // Verify no overflow with large denominators
        let a = rat(1, 1_000_000_000);
        let b = rat(1, 999_999_999);
        let sum = &a + &b;
        assert!(sum > Rat::zero());
        // Product of denoms would overflow i64; BigRational handles it
        let product = &a * &b;
        assert!(product > Rat::zero());
    }

    #[test]
    fn test_rat_from_float_large_max_denom() {
        use num_traits::ToPrimitive;
        // Verify no overflow with max_denom up to 10^18
        let r = rat_from_float(std::f64::consts::PI, 1_000_000_000_000_000_000);
        // Should be a very close approximation of pi
        let approx = r.numer().to_f64().unwrap() / r.denom().to_f64().unwrap();
        assert!((approx - std::f64::consts::PI).abs() < 1e-15);
    }

    #[test]
    fn test_rat_from_float_negative() {
        let r = rat_from_float(-0.25, 1_000_000);
        assert_eq!(r, rat(-1, 4));
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn test_rat_from_float_nan() {
        rat_from_float(f64::NAN, 1_000_000);
    }

    #[test]
    #[should_panic(expected = "must be finite")]
    fn test_rat_from_float_inf() {
        rat_from_float(f64::INFINITY, 1_000_000);
    }

}
