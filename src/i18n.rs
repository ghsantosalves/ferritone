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
