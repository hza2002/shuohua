use std::collections::BTreeSet;

pub(crate) fn extract_placeholders(template: &str) -> BTreeSet<String> {
    let mut placeholders = BTreeSet::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('}') else {
            break;
        };
        let name = &after_start[..end];
        if is_placeholder_name(name) {
            placeholders.insert(name.to_string());
        }
        rest = &after_start[end + 1..];
    }
    placeholders
}

pub(crate) fn replace_placeholders(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

pub(crate) fn pseudo_text(template: &str) -> String {
    let mut out = String::with_capacity(template.len() * 2);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&expand_text(&rest[..start]));
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('}') else {
            out.push_str(&expand_text(&rest[start..]));
            return out;
        };
        let name = &after_start[..end];
        if is_placeholder_name(name) {
            out.push('{');
            out.push_str(name);
            out.push('}');
        } else {
            out.push_str(&expand_text(&rest[start..start + end + 2]));
        }
        rest = &after_start[end + 1..];
    }
    out.push_str(&expand_text(rest));
    format!("[!! {out} !!]")
}

fn expand_text(text: &str) -> String {
    text.chars().map(accent_char).collect::<String>() + text
}

fn accent_char(ch: char) -> char {
    match ch {
        'A' => 'Á',
        'B' => 'Ɓ',
        'C' => 'Ç',
        'D' => 'Ð',
        'E' => 'É',
        'F' => 'Ƒ',
        'G' => 'Ĝ',
        'H' => 'Ĥ',
        'I' => 'Í',
        'J' => 'Ĵ',
        'K' => 'Ķ',
        'L' => 'Ļ',
        'M' => 'Ṁ',
        'N' => 'Ñ',
        'O' => 'Ó',
        'P' => 'Ƥ',
        'Q' => 'Ɋ',
        'R' => 'Ŕ',
        'S' => 'Š',
        'T' => 'Ŧ',
        'U' => 'Ú',
        'V' => 'Ṽ',
        'W' => 'Ŵ',
        'X' => 'Ẋ',
        'Y' => 'Ý',
        'Z' => 'Ž',
        'a' => 'á',
        'b' => 'ƀ',
        'c' => 'ç',
        'd' => 'ð',
        'e' => 'é',
        'f' => 'ƒ',
        'g' => 'ĝ',
        'h' => 'ĥ',
        'i' => 'í',
        'j' => 'ĵ',
        'k' => 'ķ',
        'l' => 'ļ',
        'm' => 'ṁ',
        'n' => 'ñ',
        'o' => 'ó',
        'p' => 'ƥ',
        'q' => 'ɋ',
        'r' => 'ŕ',
        's' => 'š',
        't' => 'ŧ',
        'u' => 'ú',
        'v' => 'ṽ',
        'w' => 'ŵ',
        'x' => 'ẋ',
        'y' => 'ý',
        'z' => 'ž',
        _ => ch,
    }
}

fn is_placeholder_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
