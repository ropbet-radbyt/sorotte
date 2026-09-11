pub(crate) fn localized_startup_compatibility_heading(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Syncplay-ConfigurationGetter Startkompatibilitaet:",
        Some("es") => "Compatibilidad de inicio de Syncplay ConfigurationGetter:",
        Some("eo") => "Ekfunkcia kongrueco de Syncplay ConfigurationGetter:",
        Some("fi") => "Syncplay ConfigurationGetterin kaynnistysyhteensopivuus:",
        Some("fr") => "Compatibilite de demarrage de Syncplay ConfigurationGetter :",
        Some("it") => "Compatibilita di avvio di Syncplay ConfigurationGetter:",
        Some("pt_PT" | "pt_BR") => {
            "Compatibilidade de inicializacao do Syncplay ConfigurationGetter:"
        }
        Some("tr") => "Syncplay ConfigurationGetter baslangic uyumlulugu:",
        Some("ru") => "Sovmestimost zapuska Syncplay ConfigurationGetter:",
        Some("zh_CN") => "Syncplay ConfigurationGetter qidong jianrongxing:",
        Some("ko") => "Syncplay ConfigurationGetter sijak hohwanseong:",
        _ => "Syncplay ConfigurationGetter Startup Compatibility:",
    }
}

pub(crate) fn localized_syncplay_ini_compatibility_heading(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Syncplay-ConfigurationGetter sorotte.ini-Kompatibilitaet:",
        Some("es") => "Compatibilidad sorotte.ini de Syncplay ConfigurationGetter:",
        Some("eo") => "sorotte.ini-kongrueco de Syncplay ConfigurationGetter:",
        Some("fi") => "Syncplay ConfigurationGetterin sorotte.ini-yhteensopivuus:",
        Some("fr") => "Compatibilite sorotte.ini de Syncplay ConfigurationGetter :",
        Some("it") => "Compatibilita sorotte.ini di Syncplay ConfigurationGetter:",
        Some("pt_PT" | "pt_BR") => "Compatibilidade sorotte.ini do Syncplay ConfigurationGetter:",
        Some("tr") => "Syncplay ConfigurationGetter sorotte.ini uyumlulugu:",
        Some("ru") => "Sovmestimost sorotte.ini Syncplay ConfigurationGetter:",
        Some("zh_CN") => "Syncplay ConfigurationGetter sorotte.ini jianrongxing:",
        Some("ko") => "Syncplay ConfigurationGetter sorotte.ini hohwanseong:",
        _ => "Syncplay ConfigurationGetter sorotte.ini Compatibility:",
    }
}

pub(crate) fn localized_compatibility_input_label(language: Option<&str>) -> &'static str {
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

pub(crate) fn localized_compatibility_field_label(language: Option<&str>) -> &'static str {
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

pub(crate) fn localized_compatibility_status_label(language: Option<&str>) -> &'static str {
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

pub(crate) fn localized_compatibility_note_label(language: Option<&str>) -> &'static str {
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
