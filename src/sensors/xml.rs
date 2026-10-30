//! Reads single values from the small XML answers of device APIs. This is no parser: the answers
//! are flat, and the sensors ask for tags they know.

use std::str::FromStr;

/// The text of the first element `tag`, trimmed and with entities resolved. An element written
/// as `<tag/>` counts as missing.
pub fn text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let len = xml[start..].find(&format!("</{tag}>"))?;
    Some(decode(xml[start..start + len].trim()))
}

/// The value of the first element `tag` as a number.
pub fn number<T: FromStr>(xml: &str, tag: &str) -> Option<T> {
    text(xml, tag)?.parse().ok()
}

/// Resolves the predefined and numeric entities in one pass, so `&amp;#x2F;` stays `&#x2F;`.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match rest
            .find(';')
            .and_then(|end| Some((entity(&rest[1..end])?, end)))
        {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code).filter(|c| !c.is_control())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_values_by_exact_tag() {
        let xml =
            "<r><NewUptimeX>1</NewUptimeX><NewUptime> 42 </NewUptime><Empty></Empty><Short/></r>";
        assert_eq!(text(xml, "NewUptime").as_deref(), Some("42"));
        assert_eq!(number::<u64>(xml, "NewUptime"), Some(42));
        assert_eq!(text(xml, "Empty").as_deref(), Some(""));
        assert_eq!(number::<u64>(xml, "Empty"), None);
        assert_eq!(text(xml, "Short"), None);
        assert_eq!(text(xml, "Missing"), None);
        assert_eq!(text("<a>never closed", "a"), None);
    }

    #[test]
    fn resolves_entities_once() {
        for (raw, expected) in [
            ("a&#x2F;b", "a/b"),
            ("a&#47;b", "a/b"),
            ("&lt;x&gt; &amp; &quot;y&quot; &apos;", "<x> & \"y\" '"),
            ("&amp;#x2F;", "&#x2F;"),
            ("AT&T", "AT&T"),
            ("&unknown; &#xZZ; &#x1;", "&unknown; &#xZZ; &#x1;"),
            ("ends with &", "ends with &"),
            ("ä&#xE4;", "ää"),
        ] {
            assert_eq!(decode(raw), expected, "{raw}");
        }
    }
}
