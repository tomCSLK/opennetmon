//! Symbols of the page frame (header, user menu, settings, status), as inline SVG. Own simple shapes in the
//! stroke style of the icons in `templates/icons.html`; the frame uses them through [`icon`].

/// The SVG of the symbol `name` (a dot for an unknown name); `class` adds classes such as `lg`.
/// The result is markup: render it with `|safe` (it contains no user input).
pub fn icon(name: impl AsRef<str>, class: &str) -> String {
    let class: String = class
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ' ')
        .collect();
    format!(
        "<svg class=\"icon {class}\" viewBox=\"0 0 24 24\" aria-hidden=\"true\">{}</svg>",
        shapes(name.as_ref())
    )
}

fn shapes(name: &str) -> &'static str {
    match name {
        "overview" => {
            r#"<rect x="3" y="3" width="7" height="9" rx="1"/><rect x="14" y="3" width="7" height="5" rx="1"/><rect x="14" y="12" width="7" height="9" rx="1"/><rect x="3" y="16" width="7" height="5" rx="1"/>"#
        }
        "log" => {
            r#"<rect x="5" y="3" width="14" height="18" rx="2"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="9" y1="12" x2="15" y2="12"/><line x1="9" y1="16" x2="13" y2="16"/>"#
        }
        "reports" => {
            r#"<polyline points="3 3 3 21 21 21"/><line x1="8" y1="17" x2="8" y2="13"/><line x1="13" y1="17" x2="13" y2="6"/><line x1="18" y1="17" x2="18" y2="10"/>"#
        }
        "changes" => {
            r#"<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6"/><polyline points="3 3 3 8 8 8"/><polyline points="12 7.5 12 12 15 14"/>"#
        }
        "search" => r#"<circle cx="11" cy="11" r="7"/><line x1="20.5" y1="20.5" x2="16" y2="16"/>"#,
        "up" => r#"<circle cx="12" cy="12" r="9"/><polyline points="8.5 12.5 11 15 15.5 9.5"/>"#,
        "warning" => {
            r#"<path d="M12 3.5 2.8 19.5h18.4z"/><line x1="12" y1="10" x2="12" y2="14"/><line x1="12" y1="17" x2="12.01" y2="17"/>"#
        }
        "down" => {
            r#"<circle cx="12" cy="12" r="9"/><line x1="9" y1="9" x2="15" y2="15"/><line x1="15" y1="9" x2="9" y2="15"/>"#
        }
        "inactive" => {
            r#"<circle cx="12" cy="12" r="9"/><line x1="10" y1="9" x2="10" y2="15"/><line x1="14" y1="9" x2="14" y2="15"/>"#
        }
        "all" => r#"<polyline points="2 12 6 12 9 4 15 20 18 12 22 12"/>"#,
        "user" => r#"<circle cx="12" cy="8" r="4.5"/><path d="M4 21a8 8 0 0 1 16 0"/>"#,
        "settings" => {
            r#"<line x1="4" y1="7" x2="11" y2="7"/><line x1="17" y1="7" x2="20" y2="7"/><circle cx="14" cy="7" r="2.6"/><line x1="4" y1="17" x2="7" y2="17"/><line x1="13" y1="17" x2="20" y2="17"/><circle cx="10" cy="17" r="2.6"/>"#
        }
        "sun" => {
            r#"<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>"#
        }
        "moon" => r#"<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>"#,
        "keyboard" => {
            r#"<rect x="2.5" y="5" width="19" height="14" rx="2"/><path d="M6.5 9h.01M10 9h.01M14 9h.01M17.5 9h.01M8 12.5h.01M12 12.5h.01M16 12.5h.01"/><line x1="7.5" y1="16" x2="16.5" y2="16"/>"#
        }
        "logout" => {
            r#"<path d="M9 20H5.5a1.5 1.5 0 0 1-1.5-1.5v-13A1.5 1.5 0 0 1 5.5 4H9"/><polyline points="15.5 16.5 20 12 15.5 7.5"/><line x1="20" y1="12" x2="9" y2="12"/>"#
        }
        "update" => {
            r#"<circle cx="12" cy="12" r="9"/><polyline points="8 12 12 8 16 12"/><line x1="12" y1="8" x2="12" y2="16.5"/>"#
        }
        "external" => {
            r#"<path d="M18 13.5V19a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 19V7.5A1.5 1.5 0 0 1 5.5 6H11"/><polyline points="15 3.5 20.5 3.5 20.5 9"/><line x1="11" y1="13" x2="20.5" y2="3.5"/>"#
        }
        "close" => r#"<line x1="6" y1="6" x2="18" y2="18"/><line x1="18" y1="6" x2="6" y2="18"/>"#,
        "credentials" => {
            r#"<circle cx="8" cy="15" r="4.5"/><line x1="11.2" y1="11.8" x2="20" y2="3"/><line x1="16.5" y1="6.5" x2="19.5" y2="9.5"/><line x1="14.5" y1="8.5" x2="16.5" y2="10.5"/>"#
        }
        "users" => {
            r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M16 4.6a3.5 3.5 0 0 1 0 6.8"/><path d="M18.5 14.4A6.5 6.5 0 0 1 21.5 20"/>"#
        }
        "mail" => {
            r#"<rect x="2.5" y="5" width="19" height="14" rx="2"/><polyline points="3 7 12 13 21 7"/>"#
        }
        "templates" => {
            r#"<rect x="3" y="3" width="18" height="7" rx="1"/><rect x="3" y="14" width="9" height="7" rx="1"/><rect x="16" y="14" width="5" height="7" rx="1"/>"#
        }
        "schedules" => {
            r#"<rect x="3" y="4.5" width="18" height="16.5" rx="2"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="8" y1="2.5" x2="8" y2="6.5"/><line x1="16" y1="2.5" x2="16" y2="6.5"/>"#
        }
        "api" => {
            r#"<polyline points="8 7 3 12 8 17"/><polyline points="16 7 21 12 16 17"/><line x1="14" y1="5" x2="10" y2="19"/>"#
        }
        "mibs" => {
            r#"<path d="M5 3h9.5L19 7.5V21H5z"/><polyline points="14 3 14 8 19 8"/><line x1="8.5" y1="13" x2="15.5" y2="13"/><line x1="8.5" y1="17" x2="13" y2="17"/>"#
        }
        "updates" => {
            r#"<path d="M20 12a8 8 0 0 1-13.7 5.6"/><path d="M4 12a8 8 0 0 1 13.7-5.6"/><polyline points="18 3 18 7 14 7"/><polyline points="6 21 6 17 10 17"/>"#
        }
        _ => r#"<circle cx="12" cy="12" r="3"/>"#,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_symbols() {
        assert!(icon("up", "lg").starts_with("<svg class=\"icon lg\""));
        assert!(icon("up", "").contains("polyline"));
        assert!(icon("nope", "").contains("r=\"3\""));
        assert!(!icon("up", "\"><script>").contains("<script"));
    }
}
