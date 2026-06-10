//! Konvertering av svenska talord till siffror, t.ex.
//! "tjugofem" -> "25", "etthundratjugotre" -> "123", "femtusen" -> "5000".
//!
//! Fristående "en"/"ett" lämnas orörda eftersom de oftast är artiklar
//! ("en katt" ska inte bli "1 katt").

const PARTS: &[(&str, u64)] = &[
    ("noll", 0),
    ("en", 1),
    ("ett", 1),
    ("två", 2),
    ("tre", 3),
    ("fyra", 4),
    ("fem", 5),
    ("sex", 6),
    ("sju", 7),
    ("åtta", 8),
    ("nio", 9),
    ("tio", 10),
    ("elva", 11),
    ("tolv", 12),
    ("tretton", 13),
    ("fjorton", 14),
    ("femton", 15),
    ("sexton", 16),
    ("sjutton", 17),
    ("arton", 18),
    ("aderton", 18),
    ("nitton", 19),
    ("tjugo", 20),
    ("trettio", 30),
    ("fyrtio", 40),
    ("femtio", 50),
    ("sextio", 60),
    ("sjuttio", 70),
    ("åttio", 80),
    ("nittio", 90),
    ("hundra", 100),
    ("tusen", 1_000),
    ("miljon", 1_000_000),
    ("miljoner", 1_000_000),
    ("miljard", 1_000_000_000),
    ("miljarder", 1_000_000_000),
];

/// Tolkar ett sammansatt svenskt talord ("tjugofemtusen"). Returnerar None
/// om strängen inte är ett rent talord.
fn parse_sv_number(word: &str) -> Option<u64> {
    let mut total: u64 = 0;
    let mut current: u64 = 0;
    let mut matched = false;
    let mut rest = word;
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        // "och" kan förekomma inuti tal: "hundraochfem".
        if let Some(r) = rest.strip_prefix("och") {
            rest = r;
            continue;
        }
        // Längsta matchande morfem vinner ("sjuttio" före "sju").
        let (w, v) = PARTS
            .iter()
            .filter(|(w, _)| rest.starts_with(w))
            .max_by_key(|(w, _)| w.len())?;
        match *v {
            100 => current = if current == 0 { 100 } else { current * 100 },
            1_000 | 1_000_000 | 1_000_000_000 => {
                let mult = if current == 0 { 1 } else { current };
                total = total.checked_add(mult.checked_mul(*v)?)?;
                current = 0;
            }
            n => current = current.checked_add(n)?,
        }
        matched = true;
        rest = &rest[w.len()..];
    }
    // Ordet måste innehålla minst ett talmorfem ("och" ensamt är inget tal).
    if !matched {
        return None;
    }
    total.checked_add(current)
}

/// Ersätter fristående talord i en text med siffror. Skiljetecken runt
/// orden bevaras.
pub fn convert_sv(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for token in text.split(' ') {
        if !first {
            out.push(' ');
        }
        first = false;

        // Dela upp i prefix (t.ex. citattecken), kärna och suffix (t.ex. ".").
        let core_start = token
            .char_indices()
            .find(|(_, c)| c.is_alphabetic())
            .map(|(i, _)| i);
        let Some(start) = core_start else {
            out.push_str(token);
            continue;
        };
        let end = token
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_alphabetic())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(token.len());
        let (prefix, rest) = token.split_at(start);
        let (core, suffix) = rest.split_at(end - start);

        let lower = core.to_lowercase();
        // Fristående "en"/"ett" är nästan alltid artiklar - rör dem inte.
        if lower == "en" || lower == "ett" {
            out.push_str(token);
            continue;
        }
        match parse_sv_number(&lower) {
            Some(n) => {
                out.push_str(prefix);
                out.push_str(&n.to_string());
                out.push_str(suffix);
            }
            None => out.push_str(token),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enkla_tal() {
        assert_eq!(parse_sv_number("fem"), Some(5));
        assert_eq!(parse_sv_number("tolv"), Some(12));
        assert_eq!(parse_sv_number("tjugo"), Some(20));
        assert_eq!(parse_sv_number("noll"), Some(0));
    }

    #[test]
    fn sammansatta_tal() {
        assert_eq!(parse_sv_number("tjugofem"), Some(25));
        assert_eq!(parse_sv_number("fyrtiosju"), Some(47));
        assert_eq!(parse_sv_number("etthundratjugotre"), Some(123));
        assert_eq!(parse_sv_number("hundrafem"), Some(105));
        assert_eq!(parse_sv_number("femtusen"), Some(5000));
        assert_eq!(parse_sv_number("tjugofemtusentrehundra"), Some(25_300));
        assert_eq!(parse_sv_number("tvåmiljoner"), Some(2_000_000));
        assert_eq!(parse_sv_number("hundraochfem"), Some(105));
    }

    #[test]
    fn inte_tal() {
        assert_eq!(parse_sv_number("sjukhus"), None);
        assert_eq!(parse_sv_number("trean"), None);
        assert_eq!(parse_sv_number("internet"), None);
        assert_eq!(parse_sv_number("ettor"), None);
    }

    #[test]
    fn text_konvertering() {
        assert_eq!(
            convert_sv("Han köpte tjugofem äpplen."),
            "Han köpte 25 äpplen."
        );
        assert_eq!(convert_sv("en katt och ett hus"), "en katt och ett hus");
        assert_eq!(
            convert_sv("Tjugofem personer kom klockan fjorton."),
            "25 personer kom klockan 14."
        );
        assert_eq!(
            convert_sv("Det kostar femtusen kronor."),
            "Det kostar 5000 kronor."
        );
    }
}
