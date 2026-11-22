//! Web-safe file names: `Photo Équipe (1).JPG` → `photo-equipe-1`.

pub fn slugify(name: &str) -> String {
    let ascii = deunicode::deunicode(name);
    let mut out = String::with_capacity(ascii.len());
    let mut dash = false;
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > 80 {
        out.truncate(80);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() { "image".to_owned() } else { out }
}

/// Keeps a name usable on every file system when slugs are off.
pub fn sanitize(name: &str) -> String {
    let cleaned: String =
        name.chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_owned();
    if trimmed.is_empty() { "image".to_owned() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Photo Équipe (1)"), "photo-equipe-1");
        assert_eq!(slugify("IMG_2034"), "img-2034");
        assert_eq!(slugify("  --Café crème--  "), "cafe-creme");
        assert_eq!(slugify("北京"), "bei-jing");
        assert_eq!(slugify("***"), "image");
        assert_eq!(sanitize("a/b:c"), "a_b_c");
    }
}
