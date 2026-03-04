pub const SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY: &str =
    "de/en/es/eo/fi/fr/it/pt_PT/pt_BR/tr/ru/zh_CN/ko";

pub fn normalized_legacy_runtime_language_tag_legacy_compatible(
    language: &str,
) -> Option<&'static str> {
    let normalized = language.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "de" => Some("de"),
        "en" => Some("en"),
        "es" => Some("es"),
        "eo" => Some("eo"),
        "fi" => Some("fi"),
        "fr" => Some("fr"),
        "it" => Some("it"),
        "pt_pt" => Some("pt_PT"),
        "pt_br" => Some("pt_BR"),
        "tr" => Some("tr"),
        "ru" => Some("ru"),
        "zh_cn" => Some("zh_CN"),
        "ko" => Some("ko"),
        _ => None,
    }
}

pub fn resolve_legacy_runtime_language_tag_legacy_compatible(
    cli_language: Option<&str>,
    stored_language: Option<&str>,
) -> Option<&'static str> {
    cli_language
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .or_else(|| {
            stored_language.and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        })
}

pub fn legacy_runtime_language_acknowledgement_line_legacy_compatible(
    language: &str,
) -> Option<&'static str> {
    match normalized_legacy_runtime_language_tag_legacy_compatible(language)? {
        "de" => Some(
            "hinweis: Syncplay-Sprache aktiv: Deutsch (de). syncplay-cli normalisiert und speichert diese Sprache jetzt; die meisten Laufzeittexte bleiben noch Englisch.",
        ),
        "en" => Some(
            "note: Syncplay language active: English (en). syncplay-cli now normalizes and stores this language; most runtime text still uses English.",
        ),
        "es" => Some(
            "nota: idioma de Syncplay activo: Espanol (es). syncplay-cli ahora normaliza y guarda este idioma; la mayoria de los textos en ejecucion siguen en ingles.",
        ),
        "eo" => Some(
            "noto: Syncplay-lingvo aktiva: Esperanto (eo). syncplay-cli nun normaligas kaj konservas ci tiun lingvon; plej multaj rultekstoj ankorau restas en la angla.",
        ),
        "fi" => Some(
            "huomio: Syncplay-kieli aktiivinen: Suomi (fi). syncplay-cli normalisoi ja tallentaa taman kielen nyt; suurin osa ajonaikaisesta tekstista on edelleen englanniksi.",
        ),
        "fr" => Some(
            "note: langue Syncplay active : Francais (fr). syncplay-cli normalise et enregistre desormais cette langue ; la plupart des textes d'execution restent encore en anglais.",
        ),
        "it" => Some(
            "nota: lingua di Syncplay attiva: Italiano (it). syncplay-cli ora normalizza e salva questa lingua; la maggior parte dei testi di esecuzione resta ancora in inglese.",
        ),
        "pt_PT" => Some(
            "nota: idioma do Syncplay ativo: Portugues (Portugal) (pt_PT). syncplay-cli agora normaliza e guarda este idioma; a maioria dos textos em execucao continua em ingles.",
        ),
        "pt_BR" => Some(
            "nota: idioma do Syncplay ativo: Portugues (Brasil) (pt_BR). syncplay-cli agora normaliza e salva este idioma; a maioria dos textos em execucao continua em ingles.",
        ),
        "tr" => Some(
            "not: Syncplay dili etkin: Turkce (tr). syncplay-cli artik bu dili normallestirip kaydediyor; calisma zamani metinlerinin cogu hala Ingilizce.",
        ),
        "ru" => Some(
            "primechanie: iazyk Syncplay aktiven: Russkii (ru). syncplay-cli teper normalizuet i sokhraniaet etot iazyk; bolshaia chast teksta vo vremia raboty poka ostaetsia na angliiskom.",
        ),
        "zh_CN" => Some(
            "note: Syncplay yuyan yi qiyong: Chinese (Simplified) (zh_CN). syncplay-cli xianzai hui guifanhua bing baocun gai yuyan; da duoshu yunxing shi wenben rengran shi yingwen.",
        ),
        "ko" => Some(
            "note: Syncplay eoneo hwalseonghwa: Korean (ko). syncplay-cli-ga ije i eoneoreul jeonggyuhwa hae jeojanghajiman, daebubun-ui silhaeng jung tekstneun ajik yeongeoimnida.",
        ),
        _ => None,
    }
}

pub fn legacy_runtime_language_selection_line_legacy_compatible(
    language: Option<&str>,
) -> Option<String> {
    let language = language?;
    if let Some(line) = legacy_runtime_language_acknowledgement_line_legacy_compatible(language) {
        return Some(line.to_owned());
    }
    Some(format!(
        "warning: unsupported legacy --language value '{}' was ignored; supported values: {}",
        language.trim(),
        SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
        legacy_runtime_language_selection_line_legacy_compatible,
        normalized_legacy_runtime_language_tag_legacy_compatible,
        resolve_legacy_runtime_language_tag_legacy_compatible,
    };

    #[test]
    fn normalized_legacy_runtime_language_tag_accepts_python_tags_and_aliases() {
        assert_eq!(
            normalized_legacy_runtime_language_tag_legacy_compatible("fr"),
            Some("fr")
        );
        assert_eq!(
            normalized_legacy_runtime_language_tag_legacy_compatible("PT-br"),
            Some("pt_BR")
        );
        assert_eq!(
            normalized_legacy_runtime_language_tag_legacy_compatible("zh-cn"),
            Some("zh_CN")
        );
        assert_eq!(
            normalized_legacy_runtime_language_tag_legacy_compatible("klingon"),
            None
        );
    }

    #[test]
    fn resolve_legacy_runtime_language_tag_prefers_cli_and_falls_back_to_stored() {
        assert_eq!(
            resolve_legacy_runtime_language_tag_legacy_compatible(Some("PT-br"), Some("fr")),
            Some("pt_BR")
        );
        assert_eq!(
            resolve_legacy_runtime_language_tag_legacy_compatible(Some("klingon"), Some("fr")),
            Some("fr")
        );
    }

    #[test]
    fn selection_line_warns_for_invalid_values_and_lists_supported_tags() {
        let invalid = legacy_runtime_language_selection_line_legacy_compatible(Some("klingon"))
            .expect("expected warning line");
        assert!(invalid.contains("unsupported legacy --language value"));
        assert!(invalid.contains(SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY));
    }
}
