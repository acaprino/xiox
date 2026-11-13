//! Sizes typed by a person: `2GB`, `1.5 GiB`, `500mb`, `1024`.

/// Bytes of a size text. Units are binary (`KB` = `KiB` = 1024 bytes): `B`, `KB`/`KiB`, `MB`/`MiB`,
/// `GB`/`GiB`, `TB`/`TiB`, in any case, with or without a space; no unit means bytes. Zero,
/// negative, empty and garbage texts are errors, and so is a size that does not fit in 64 bits.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let bad = || format!("invalid size \"{text}\" (examples: 500MB, 2GB, 1.5GiB, 1073741824)");
    let trimmed = text.trim();
    let digits_end = trimmed.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(digits_end);
    let multiplier: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" | "kib" => 1 << 10,
        "mb" | "mib" => 1 << 20,
        "gb" | "gib" => 1 << 30,
        "tb" | "tib" => 1 << 40,
        _ => return Err(bad()),
    };
    // Digits, optionally followed by a dot and more digits: nothing else is a number here.
    let (whole, fraction) = match number.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (number, None),
    };
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || fraction.is_some_and(|f| !all_digits(f)) {
        return Err(bad());
    }
    let bytes = match fraction {
        None => {
            let n: u64 = whole.parse().map_err(|_| bad())?;
            n.checked_mul(multiplier).ok_or_else(bad)?
        }
        Some(_) => {
            let n: f64 = number.parse().map_err(|_| bad())?;
            let total = n * multiplier as f64;
            if !total.is_finite() || total >= u64::MAX as f64 {
                return Err(bad());
            }
            total as u64
        }
    };
    if bytes == 0 {
        return Err(bad());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    #[test]
    fn parses_units_case_insensitively_and_binary() {
        let cases: [(&str, u64); 23] = [
            ("1024", 1024),
            ("1", 1),
            ("7B", 7),
            ("7 b", 7),
            ("1KB", KIB),
            ("1KiB", KIB),
            ("1kib", KIB),
            ("500MB", 500 * MIB),
            ("500mb", 500 * MIB),
            ("500 MiB", 500 * MIB),
            ("2GB", 2 * GIB),
            ("2 GiB", 2 * GIB),
            ("2gib", 2 * GIB),
            ("2Gb", 2 * GIB),
            ("1TB", TIB),
            ("1 tib", TIB),
            ("1.5GB", GIB + GIB / 2),
            ("1.5 GiB", GIB + GIB / 2),
            ("0.5MB", MIB / 2),
            ("  3GB  ", 3 * GIB),
            ("3\tGB", 3 * GIB),
            ("2.25KB", 2 * KIB + KIB / 4),
            ("18446744073709551615", u64::MAX),
        ];
        for (text, want) in cases {
            assert_eq!(parse_size(text), Ok(want), "{text}");
        }
    }

    #[test]
    fn rejects_zero_negative_empty_and_garbage() {
        for text in [
            "", "   ", "0", "0GB", "0.0GB", "00", "-1", "-2GB", "+", "GB", "banana", "2XB", "2 G B", "1..5GB", ".5GB",
            "5.GB", "1e3", "1,5GB", "NaN", "inf", "2GB3", "2GBGB", "1 2", "18446744073709551616",
            "99999999999TB", "0.0000001B",
        ] {
            let r = parse_size(text);
            assert!(r.is_err(), "{text:?} must be refused, got {r:?}");
            assert!(r.unwrap_err().contains("invalid size"), "{text:?}");
        }
    }
}
