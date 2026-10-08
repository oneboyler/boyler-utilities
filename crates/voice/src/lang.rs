//! Speech languages: Windows names its dictation languages by BCP-47 tag ("en-US"); the page shows a short name.

/// One language Windows can take dictation in on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// BCP-47 tag, e.g. "en-US" (the setting the page stores)
    pub tag: String,
    /// what the language picker shows, e.g. "English (US)"
    pub name: String,
    /// which engine offers it (real: "online"; fake: any id)
    pub engine: String,
}

/// The picker's short name (the drawing: "English (US)", "English (UK)", "Deutsch"); a language not in the list keeps its tag
/// (the real layer then uses Windows' own name for it).
pub fn short_name(tag: &str) -> String {
    match tag {
        "en-US" => "English (US)",
        "en-GB" => "English (UK)",
        "en-AU" => "English (AU)",
        "en-CA" => "English (CA)",
        "en-IN" => "English (India)",
        "de-DE" => "Deutsch",
        "fr-FR" => "Français",
        "fr-CA" => "Français (CA)",
        "es-ES" => "Español",
        "es-MX" => "Español (MX)",
        "it-IT" => "Italiano",
        "pt-BR" => "Português (BR)",
        "pt-PT" => "Português",
        "ja-JP" => "日本語",
        "zh-CN" => "中文 (简体)",
        "zh-TW" => "中文 (繁體)",
        "hr-HR" => "Hrvatski",
        "pl-PL" => "Polski",
        "ru-RU" => "Русский",
        "nl-NL" => "Nederlands",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_and_unknown_tags_kept() {
        assert_eq!(short_name("en-US"), "English (US)");
        assert_eq!(short_name("de-DE"), "Deutsch");
        assert_eq!(short_name("xx-YY"), "xx-YY");
    }
}
