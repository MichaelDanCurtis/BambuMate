//! Printer model names as people and the app write them. One place decides
//! that "Bambu Lab X1 Carbon", "X1C" and "x1 carbon" are the same printer.

use super::discovery::model_name;

/// Letters and digits only, uppercased: "A1 mini" and "A1MINI" compare equal.
pub(crate) fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// A hand-typed model with the maker and bundle words removed:
/// "Bambu Lab X1 Carbon" becomes "X1 Carbon", "P1S Combo" becomes "P1S".
pub(crate) fn clean_model(model: &str) -> String {
    const LEADING: &[&str] = &["bambu", "lab", "bambulab"];
    const TRAILING: &[&str] = &["combo", "ams", "with", "printer", "lite"];
    let mut words: Vec<&str> = model.split_whitespace().collect();
    while words
        .first()
        .is_some_and(|w| LEADING.contains(&w.to_ascii_lowercase().as_str()))
    {
        words.remove(0);
    }
    while words
        .last()
        .is_some_and(|w| TRAILING.contains(&w.to_ascii_lowercase().as_str()))
    {
        words.pop();
    }
    words.join(" ")
}

/// The model without a trailing `0.4 nozzle`: "H2D 0.4 nozzle" is "H2D".
fn without_nozzle(model: &str) -> String {
    let mut words: Vec<&str> = model.split_whitespace().collect();
    if words
        .last()
        .is_some_and(|w| w.eq_ignore_ascii_case("nozzle"))
    {
        words.pop();
        if words.last().is_some_and(|w| w.parse::<f64>().is_ok()) {
            words.pop();
        }
    }
    words.join(" ")
}

/// A key equal for every spelling of one printer: maker and bundle words,
/// a nozzle suffix, case and spacing are ignored; discovery codes (`O1C2`)
/// and short names (`X1C`, `A1M`, `H2DP`) become the model's full name. H2D
/// and H2D Pro stay different printers.
pub(crate) fn model_key(model: &str) -> String {
    let cleaned = clean_model(&without_nozzle(&clean_model(model)));
    let key = squash(&model_name(&cleaned));
    match key.as_str() {
        "X1C" => "X1CARBON".to_string(),
        "A1M" => "A1MINI".to_string(),
        "H2DP" => "H2DPRO".to_string(),
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_printer_has_one_key() {
        for same in [
            &[
                "X1C",
                "X1 Carbon",
                "Bambu Lab X1 Carbon",
                "x1carbon",
                "BL-P001",
            ][..],
            &["A1M", "A1 mini", "Bambu Lab A1 mini 0.4 nozzle", "N1"],
            &[
                "H2C",
                "Bambu Lab H2C",
                "h2c combo",
                "O1C2",
                "H2C 0.6 nozzle",
            ],
            &["H2D Pro", "H2DP", "Bambu Lab H2D Pro", "O1E"],
            &["P1S", "P1S Combo", "Bambu Lab P1S", "C12"],
        ] {
            let keys: Vec<String> = same.iter().map(|m| model_key(m)).collect();
            assert!(keys.iter().all(|k| *k == keys[0]), "{same:?} -> {keys:?}");
        }
        assert_ne!(model_key("H2D"), model_key("H2D Pro"));
        assert_ne!(model_key("A1"), model_key("A1 mini"));
        assert_eq!(model_key(""), "");
    }
}
