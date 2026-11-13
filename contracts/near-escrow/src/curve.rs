//! Pallas checks for the two key shares.
//!
//! Encodings (the same as `pasta_curves` and the Orchard client):
//! - A point is the 32-byte compressed Pallas encoding: the x coordinate as a 32-byte
//!   little-endian integer, with bit 255 (the top bit of byte 31) set to the parity of y.
//!   Only canonical encodings decode (x < p, and exactly one encoding per point).
//! - A scalar is 32 bytes, little-endian, canonical (less than the group order q).

use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::group::{Group, GroupEncoding};
use pasta_curves::pallas;

/// x of G^Orchard (RedPallas SpendAuth basepoint), 32 bytes little-endian.
/// Big-endian it is 0x375523b328f1d6063b8d187c3e5f445f0c7f0ce37b70a10c8d1a7284b875c963,
/// the `GX` constant of `contracts/src/Pallas.sol`.
pub const G_X: [u8; 32] = [
    0x63, 0xc9, 0x75, 0xb8, 0x84, 0x72, 0x1a, 0x8d, 0x0c, 0xa1, 0x70, 0x7b, 0xe3, 0x0c, 0x7f, 0x0c,
    0x5f, 0x44, 0x5f, 0x3e, 0x7c, 0x18, 0x8d, 0x3b, 0x06, 0xd6, 0xf1, 0x28, 0xb3, 0x23, 0x55, 0x37,
];

/// y of G^Orchard, 32 bytes little-endian.
/// Big-endian it is 0x1ad0357fdf1a66db7b10bcfcfed624fbdfc914fec005bdd84ce33e817b0c3bc9,
/// the `GY` constant of `contracts/src/Pallas.sol`.
pub const G_Y: [u8; 32] = [
    0xc9, 0x3b, 0x0c, 0x7b, 0x81, 0x3e, 0xe3, 0x4c, 0xd8, 0xbd, 0x05, 0xc0, 0xfe, 0x14, 0xc9, 0xdf,
    0xfb, 0x24, 0xd6, 0xfe, 0xfc, 0xbc, 0x10, 0x7b, 0xdb, 0x66, 0x1a, 0xdf, 0x7f, 0x35, 0xd0, 0x1a,
];

/// G^Orchard, built from its affine coordinates (no square root needed). The unit tests
/// check that (G_X, G_Y) is on the curve and equals the client's basepoint.
pub fn g() -> pallas::Point {
    let x = Option::<pallas::Base>::from(pallas::Base::from_repr(G_X));
    let y = Option::<pallas::Base>::from(pallas::Base::from_repr(G_Y));
    match (x, y) {
        (Some(x), Some(y)) => pallas::Affine::from_xy_unchecked(x, y).into(),
        // Unreachable: both constants are canonical field elements.
        _ => near_sdk::env::panic_str("G is not canonical"),
    }
}

/// Decodes a canonical compressed point that is not the identity.
pub fn parse_point(bytes: &[u8; 32]) -> Option<pallas::Point> {
    let p = Option::<pallas::Point>::from(pallas::Point::from_bytes(bytes))?;
    if bool::from(p.is_identity()) {
        return None;
    }
    Some(p)
}

/// Decodes a canonical scalar (less than q) that is not zero.
pub fn parse_scalar(bytes: &[u8; 32]) -> Option<pallas::Scalar> {
    let s = Option::<pallas::Scalar>::from(pallas::Scalar::from_repr(*bytes))?;
    if bool::from(s.is_zero()) {
        return None;
    }
    Some(s)
}

/// True if `scalar` is a canonical non-zero scalar k and k·G encodes to `point`.
/// `point` must already be a validated canonical encoding (it is validated at open).
pub fn is_discrete_log_of(scalar: &[u8; 32], point: &[u8; 32]) -> bool {
    match parse_scalar(scalar) {
        Some(k) => (g() * k).to_bytes() == *point,
        None => false,
    }
}
