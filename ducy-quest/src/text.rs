//! Numbers as the page writes them in its messages.

/// `n.toLocaleString('en-US', {maximumFractionDigits: 0})`: whole, halves
/// away from zero, in groups of three.
pub fn grouped(n: f64) -> String {
    let whole = n.abs().round();
    let digits = format!("{whole:.0}");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0.0 && whole > 0.0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Hundredths as whole dollars, without the dollar sign: `1,250`.
pub fn dollars(hundredths: f64) -> String {
    grouped(hundredths / 100.0)
}

/// A share as a whole percent: `35%`.
pub fn pct(x: f64) -> String {
    format!(
        "{}%",
        crate::dice::js_num(crate::world::js_round(100.0 * x))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_the_page_writes_them() {
        assert_eq!(grouped(1234567.0), "1,234,567");
        assert_eq!(grouped(999.5), "1,000");
        assert_eq!(grouped(-2500.0), "-2,500");
        assert_eq!(dollars(125_000.0), "1,250");
        assert_eq!(pct(0.355), "36%");
    }
}
