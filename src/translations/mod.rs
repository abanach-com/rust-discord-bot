use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::LazyLock};

const EN_JSON: &str = include_str!("../../translations/en.json");
const PL_JSON: &str = include_str!("../../translations/pl.json");
const DE_JSON: &str = include_str!("../../translations/de.json");

pub static TRANSLATIONS: LazyLock<TranslationsStore> = LazyLock::new(TranslationsStore::init);

#[derive(Clone, Debug)]
pub struct TranslationsStore {
    locales: HashMap<String, Value>,
}

impl TranslationsStore {
    pub fn init() -> Self {
        let mut locales = HashMap::new();

        // 1. Load embedded defaults
        if let Ok(v) = serde_json::from_str::<Value>(EN_JSON) {
            locales.insert("en".to_owned(), v);
        }
        if let Ok(v) = serde_json::from_str::<Value>(PL_JSON) {
            locales.insert("pl".to_owned(), v);
        }
        if let Ok(v) = serde_json::from_str::<Value>(DE_JSON) {
            locales.insert("de".to_owned(), v);
        }

        // 2. Optionally load runtime overrides from ./translations directory
        Self::load_dir_overrides(Path::new("translations"), &mut locales);
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let manifest_trans = Path::new(&manifest_dir).join("translations");
            Self::load_dir_overrides(&manifest_trans, &mut locales);
        }

        Self { locales }
    }

    fn load_dir_overrides(dir: &Path, locales: &mut HashMap<String, Value>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    let lang = stem.to_lowercase();
                    if let Ok(content) = fs::read_to_string(&path)
                        && let Ok(json_val) = serde_json::from_str::<Value>(&content)
                    {
                        locales.insert(lang, json_val);
                    }
                }
            }
        }
    }

    pub fn normalize_locale(locale: &str) -> String {
        let trimmed = locale.trim().to_lowercase();
        if trimmed.starts_with("pl") {
            "pl".to_owned()
        } else if trimmed.starts_with("de") {
            "de".to_owned()
        } else if trimmed.starts_with("en") {
            "en".to_owned()
        } else {
            trimmed.split('-').next().unwrap_or("en").to_owned()
        }
    }

    fn get_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
        let mut current = root;
        for segment in path.split('.') {
            {
                let next = current.get(segment)?;
                current = next;
            }
        }
        Some(current)
    }

    pub fn lookup(&self, locale: &str, key: &str) -> Option<String> {
        let norm = Self::normalize_locale(locale);

        // Try normalized locale
        if let Some(locale_data) = self.locales.get(&norm)
            && let Some(val) = Self::get_value(locale_data, key)
            && let Some(s) = val.as_str()
        {
            return Some(s.to_owned());
        }

        // Fallback to "en" if not found and not already en
        if norm != "en"
            && let Some(en_data) = self.locales.get("en")
            && let Some(val) = Self::get_value(en_data, key)
            && let Some(s) = val.as_str()
        {
            return Some(s.to_owned());
        }

        None
    }

    pub fn t(&self, locale: &str, key: &str) -> String {
        self.lookup(locale, key).unwrap_or_else(|| key.to_owned())
    }

    pub fn t_args(&self, locale: &str, key: &str, args: &[(&str, &str)]) -> String {
        let mut template = self.t(locale, key);
        for (placeholder, value) in args {
            let target = format!("{{{placeholder}}}");
            template = template.replace(&target, value);
        }
        template
    }

    pub fn t_map(&self, locale: &str, key: &str, args: &HashMap<String, String>) -> String {
        let mut template = self.t(locale, key);
        for (placeholder, value) in args {
            let target = format!("{{{placeholder}}}");
            template = template.replace(&target, value);
        }
        template
    }

    pub fn get_localizations(&self, key: &str) -> Vec<(String, String)> {
        let mut results = Vec::new();
        for (lang, root) in &self.locales {
            if lang == "en" {
                continue;
            }
            if let Some(val) = Self::get_value(root, key)
                && let Some(s) = val.as_str()
            {
                results.push((lang.clone(), s.to_owned()));
            }
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }

    pub fn get_command_localizations(&self, cmd_name: &str) -> Vec<(String, String)> {
        let mut results = self.get_localizations(&format!("commands.{cmd_name}.description"));
        if results.is_empty() {
            for (lang, root) in &self.locales {
                if lang == "en" {
                    continue;
                }
                if let Some(modules) = root.get("modules").and_then(|m| m.as_object()) {
                    for (_mod_id, mod_val) in modules {
                        if let Some(commands) = mod_val.get("commands").and_then(|c| c.as_object())
                            && let Some(cmd_val) = commands.get(cmd_name)
                            && let Some(desc) = cmd_val.get("description").and_then(|d| d.as_str())
                        {
                            results.push((lang.clone(), desc.to_owned()));
                            break;
                        }
                    }
                }
            }
            results.sort_by(|a, b| a.0.cmp(&b.0));
        }
        results
    }

    pub fn get_command_option_localizations(
        &self,
        cmd_name: &str,
        opt_name: &str,
    ) -> Vec<(String, String)> {
        let mut results = self.get_localizations(&format!(
            "commands.{cmd_name}.options.{opt_name}.description"
        ));
        if results.is_empty() {
            for (lang, root) in &self.locales {
                if lang == "en" {
                    continue;
                }
                if let Some(modules) = root.get("modules").and_then(|m| m.as_object()) {
                    for (_mod_id, mod_val) in modules {
                        if let Some(commands) = mod_val.get("commands").and_then(|c| c.as_object())
                            && let Some(cmd_val) = commands.get(cmd_name)
                            && let Some(options) =
                                cmd_val.get("options").and_then(|o| o.as_object())
                            && let Some(opt_val) = options.get(opt_name)
                            && let Some(desc) = opt_val.get("description").and_then(|d| d.as_str())
                        {
                            results.push((lang.clone(), desc.to_owned()));
                            break;
                        }
                    }
                }
            }
            results.sort_by(|a, b| a.0.cmp(&b.0));
        }
        results
    }

    pub fn raw_translations(&self) -> &HashMap<String, Value> {
        &self.locales
    }
}

pub fn t(locale: &str, key: &str) -> String {
    TRANSLATIONS.t(locale, key)
}

pub fn t_args(locale: &str, key: &str, args: &[(&str, &str)]) -> String {
    TRANSLATIONS.t_args(locale, key, args)
}

pub fn t_map(locale: &str, key: &str, args: &HashMap<String, String>) -> String {
    TRANSLATIONS.t_map(locale, key, args)
}

pub fn get_localizations(key: &str) -> Vec<(String, String)> {
    TRANSLATIONS.get_localizations(key)
}

pub fn get_command_localizations(cmd_name: &str) -> Vec<(String, String)> {
    TRANSLATIONS.get_command_localizations(cmd_name)
}

pub fn get_command_option_localizations(cmd_name: &str, opt_name: &str) -> Vec<(String, String)> {
    TRANSLATIONS.get_command_option_localizations(cmd_name, opt_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translations_basic_lookup_and_fallback() {
        let store = TranslationsStore::init();

        // English default
        assert_eq!(
            store.t("en", "system.command_executed"),
            "Command executed successfully."
        );

        // Polish lookup
        assert_eq!(
            store.t("pl", "system.command_executed"),
            "Polecenie zostało wykonane."
        );

        // German lookup
        assert_eq!(
            store.t("de", "system.command_executed"),
            "Befehl erfolgreich ausgeführt."
        );

        // Fallback for missing language -> returns English
        assert_eq!(
            store.t("fr", "system.command_executed"),
            "Command executed successfully."
        );

        // Fallback for missing key -> returns key itself
        assert_eq!(store.t("en", "non.existent.key"), "non.existent.key");
    }

    #[test]
    fn test_translations_with_args() {
        let store = TranslationsStore::init();

        let formatted_en = store.t_args(
            "en",
            "system.rate_limited",
            &[("command", "ping"), ("seconds", "5")],
        );
        assert_eq!(
            formatted_en,
            "Rate limit exceeded for command `/ping`. Try again in 5 s."
        );

        let formatted_pl = store.t_args(
            "pl",
            "system.rate_limited",
            &[("command", "ping"), ("seconds", "5")],
        );
        assert_eq!(
            formatted_pl,
            "Osiągnięto limit wywołań komendy `/ping`. Spróbuj ponownie za 5 s."
        );

        let formatted_de = store.t_args(
            "de",
            "system.rate_limited",
            &[("command", "ping"), ("seconds", "5")],
        );
        assert_eq!(
            formatted_de,
            "Ratenbegrenzung für Befehl `/ping` überschritten. Bitte in 5 s erneut versuchen."
        );
    }

    #[test]
    fn test_get_localizations() {
        let store = TranslationsStore::init();
        let localizations = store.get_localizations("system.command_executed");
        assert!(
            localizations
                .iter()
                .any(|(lang, text)| lang == "pl" && text == "Polecenie zostało wykonane.")
        );
        assert!(
            localizations
                .iter()
                .any(|(lang, text)| lang == "de" && text == "Befehl erfolgreich ausgeführt.")
        );
    }
}
