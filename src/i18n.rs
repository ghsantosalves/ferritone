use std::{collections::HashMap, sync::OnceLock};

static TRANSLATIONS: OnceLock<HashMap<&'static str, [&'static str; 2]>> = OnceLock::new();

pub fn lookup(english: &str, locale: usize) -> Option<&'static str> {
    TRANSLATIONS
        .get_or_init(|| {
            include_str!("i18n/catalog.tsv")
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\t');
                    Some((fields.next()?, [fields.next()?, fields.next()?]))
                })
                .collect()
        })
        .get(english)
        .map(|values| values[locale])
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    /// Every `text("English", "Português")` call must have Spanish and French entries.
    #[test]
    fn catalog_covers_every_interface_string() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in fs::read_dir(source).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let code = fs::read_to_string(&path).unwrap();
            for call in code.split(".text(").skip(1) {
                let call = call.trim_start();
                let Some(rest) = call.strip_prefix('"') else {
                    continue;
                };
                let Some(end) = rest.find('"') else { continue };
                let english = &rest[..end];
                if !rest[end + 1..].trim_start().starts_with(',') {
                    continue;
                }
                if super::lookup(english, 0).is_none() {
                    missing.push(format!("{}: {english}", path.display()));
                }
            }
        }
        assert!(missing.is_empty(), "missing translations:\n{}", missing.join("\n"));
    }
}
