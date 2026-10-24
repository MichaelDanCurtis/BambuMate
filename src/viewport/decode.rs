//! Base64 float buffers from the backend: little-endian `f32`s.

use base64::Engine;

/// Decode standard base64 into little-endian `f32`s. The decoded byte length
/// must be a multiple of 4.
pub fn decode_f32(s: &str) -> Result<Vec<f32>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())?;
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "float data is {} bytes, not a multiple of 4",
            bytes.len()
        ));
    }
    Ok(bytes
        .chunks(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn decode_groups(s: &str, size: usize, what: &str) -> Result<Vec<f32>, String> {
    let floats = decode_f32(s)?;
    if floats.len() % size != 0 {
        return Err(format!(
            "{} floats is not a whole number of {what}",
            floats.len()
        ));
    }
    Ok(floats)
}

/// Floats in groups of 3 (positions or normals).
pub fn decode_triples(s: &str) -> Result<Vec<f32>, String> {
    decode_groups(s, 3, "xyz triples")
}

/// Floats in groups of 6 (two endpoints per edge segment).
pub fn decode_segments(s: &str) -> Result<Vec<f32>, String> {
    decode_groups(s, 6, "edge segments")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn enc(v: &[f32]) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
    }

    #[test]
    fn floats_round_trip_and_lengths_are_checked() {
        assert_eq!(
            decode_triples(&enc(&[1., 2., 3., 4., 5., 6.])).unwrap(),
            vec![1., 2., 3., 4., 5., 6.]
        );
        assert!(decode_triples(&enc(&[1., 2.])).is_err());
        assert!(decode_segments(&enc(&[1., 2., 3.])).is_err());
        assert!(decode_f32("not base64!").is_err());
    }

    #[test]
    fn a_byte_length_that_is_not_a_multiple_of_four_is_refused() {
        let bytes = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3, 4, 5]);
        assert!(decode_f32(&bytes).is_err());
    }

    #[test]
    fn empty_input_is_an_empty_list() {
        assert_eq!(decode_f32("").unwrap(), Vec::<f32>::new());
    }
}
