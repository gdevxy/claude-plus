use std::collections::BTreeMap;

pub fn frontmatter(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return fields;
    }
    let body: Vec<&str> = lines.take_while(|line| line.trim_end() != "---").collect();
    let mut index = 0;
    while index < body.len() {
        let line = body[index];
        index += 1;
        if line.starts_with([' ', '\t']) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let continued: Vec<&str> = body[index..]
            .iter()
            .take_while(|next| next.trim().is_empty() || next.starts_with([' ', '\t']))
            .copied()
            .collect();
        index += continued.len();
        let value = value.trim();
        let parts = continued
            .iter()
            .map(|part| part.trim())
            .filter(|part| !part.is_empty());
        let joined = match value.chars().next() {
            Some('|') => parts.collect::<Vec<_>>().join("\n"),
            Some('>') => parts.collect::<Vec<_>>().join(" "),
            _ => std::iter::once(value)
                .chain(parts)
                .collect::<Vec<_>>()
                .join(" "),
        };
        fields.insert(key.trim().to_string(), unquoted(joined.trim()));
    }
    fields
}

fn unquoted(value: &str) -> String {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_quoted_folded_and_literal_values() {
        let text = "---\nname: tidy\ndescription: \"Use when: tidying up\"\nlong: >\n  first line\n  second line\nkept: |\n  one\n  two\nplain: starts here\n  and continues\n# a comment\n---\n# Body\nname: not a field\n";
        let fields = frontmatter(text);
        assert_eq!(fields.get("name").map(String::as_str), Some("tidy"));
        assert_eq!(
            fields.get("description").map(String::as_str),
            Some("Use when: tidying up")
        );
        assert_eq!(
            fields.get("long").map(String::as_str),
            Some("first line second line")
        );
        assert_eq!(fields.get("kept").map(String::as_str), Some("one\ntwo"));
        assert_eq!(
            fields.get("plain").map(String::as_str),
            Some("starts here and continues")
        );
        assert_eq!(fields.len(), 5);
    }

    #[test]
    fn text_without_frontmatter_has_no_fields() {
        assert!(frontmatter("# Title\nname: x\n").is_empty());
        assert!(frontmatter("").is_empty());
        assert_eq!(
            frontmatter("\u{feff}---\r\nname: bom\r\n---\r\n")
                .get("name")
                .map(String::as_str),
            Some("bom")
        );
    }
}
