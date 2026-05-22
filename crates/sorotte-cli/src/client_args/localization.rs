pub(crate) fn localized_legacy_startup_compatibility_heading_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Legacy-Python-ConfigurationGetter Startkompatibilitaet:",
        Some("es") => "Compatibilidad de inicio de Legacy Python ConfigurationGetter:",
        Some("eo") => "Ekfunkcia kongrueco de Legacy Python ConfigurationGetter:",
        Some("fi") => "Legacy Python ConfigurationGetterin kaynnistysyhteensopivuus:",
        Some("fr") => "Compatibilite de demarrage de Legacy Python ConfigurationGetter :",
        Some("it") => "Compatibilita di avvio di Legacy Python ConfigurationGetter:",
        Some("pt_PT" | "pt_BR") => {
            "Compatibilidade de inicializacao do Legacy Python ConfigurationGetter:"
        }
        Some("tr") => "Legacy Python ConfigurationGetter baslangic uyumlulugu:",
        Some("ru") => "Sovmestimost zapuska Legacy Python ConfigurationGetter:",
        Some("zh_CN") => "Legacy Python ConfigurationGetter qidong jianrongxing:",
        Some("ko") => "Legacy Python ConfigurationGetter sijak hohwanseong:",
        _ => "Legacy Python ConfigurationGetter Startup Compatibility:",
    }
}

pub(crate) fn localized_legacy_ini_compatibility_heading_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Legacy-Python-ConfigurationGetter sorotte.ini-Kompatibilitaet:",
        Some("es") => "Compatibilidad sorotte.ini de Legacy Python ConfigurationGetter:",
        Some("eo") => "sorotte.ini-kongrueco de Legacy Python ConfigurationGetter:",
        Some("fi") => "Legacy Python ConfigurationGetterin sorotte.ini-yhteensopivuus:",
        Some("fr") => "Compatibilite sorotte.ini de Legacy Python ConfigurationGetter :",
        Some("it") => "Compatibilita sorotte.ini di Legacy Python ConfigurationGetter:",
        Some("pt_PT" | "pt_BR") => {
            "Compatibilidade sorotte.ini do Legacy Python ConfigurationGetter:"
        }
        Some("tr") => "Legacy Python ConfigurationGetter sorotte.ini uyumlulugu:",
        Some("ru") => "Sovmestimost sorotte.ini Legacy Python ConfigurationGetter:",
        Some("zh_CN") => "Legacy Python ConfigurationGetter sorotte.ini jianrongxing:",
        Some("ko") => "Legacy Python ConfigurationGetter sorotte.ini hohwanseong:",
        _ => "Legacy Python ConfigurationGetter sorotte.ini Compatibility:",
    }
}

pub(crate) fn localized_compatibility_input_label_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Eingabe",
        Some("es") => "Entrada",
        Some("eo") => "Enigo",
        Some("fi") => "Syote",
        Some("fr") => "Entree",
        Some("it") => "Input",
        Some("pt_PT" | "pt_BR") => "Entrada",
        Some("tr") => "Girdi",
        Some("ru") => "Vvod",
        Some("zh_CN") => "Shuru",
        Some("ko") => "Iblyeog",
        _ => "Input",
    }
}

pub(crate) fn localized_compatibility_field_label_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Feld",
        Some("es") => "Campo",
        Some("eo") => "Kampo",
        Some("fi") => "Kenta",
        Some("fr") => "Champ",
        Some("it") => "Campo",
        Some("pt_PT" | "pt_BR") => "Campo",
        Some("tr") => "Alan",
        Some("ru") => "Pole",
        Some("zh_CN") => "Ziduan",
        Some("ko") => "Pildeu",
        _ => "Field",
    }
}

pub(crate) fn localized_compatibility_status_label_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Status",
        Some("es") => "Estado",
        Some("eo") => "Stato",
        Some("fi") => "Tila",
        Some("fr") => "Statut",
        Some("it") => "Stato",
        Some("pt_PT" | "pt_BR") => "Status",
        Some("tr") => "Durum",
        Some("ru") => "Status",
        Some("zh_CN") => "Zhuangtai",
        Some("ko") => "Sangtae",
        _ => "Status",
    }
}

pub(crate) fn localized_compatibility_note_label_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Hinweis",
        Some("es") => "Nota",
        Some("eo") => "Noto",
        Some("fi") => "Huomio",
        Some("fr") => "Note",
        Some("it") => "Nota",
        Some("pt_PT" | "pt_BR") => "Nota",
        Some("tr") => "Not",
        Some("ru") => "Primechanie",
        Some("zh_CN") => "Beizhu",
        Some("ko") => "Bigo",
        _ => "Note",
    }
}
