use syncplay_client_core::FileDifferenceSummary;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileDifferenceNotificationState {
    last_summary: Option<String>,
}

pub fn format_file_difference_summary(summary: FileDifferenceSummary) -> Option<String> {
    let mut differences = Vec::new();
    if summary.filename {
        differences.push("filename");
    }
    if summary.filesize {
        differences.push("filesize");
    }
    if summary.fileduration {
        differences.push("duration");
    }

    if differences.is_empty() {
        None
    } else {
        Some(differences.join(", "))
    }
}

pub fn localized_file_differences_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Dateiunterschiede",
        Some("es") => "Diferencias de archivo",
        Some("eo") => "Dosieraj diferencoj",
        Some("fi") => "Tiedostoerot",
        Some("fr") => "Differences de fichier",
        Some("it") => "Differenze del file",
        Some("pt_PT" | "pt_BR") => "Diferencas de arquivo",
        Some("tr") => "Dosya farklari",
        Some("ru") => "Razlichiia failov",
        Some("zh_CN") => "Wenjian chayi",
        Some("ko") => "Pail chai",
        _ => "file differences",
    }
}

fn localized_file_difference_token_legacy_compatible(
    token: &str,
    language: Option<&str>,
) -> String {
    match (token, language) {
        ("filename", Some("de")) => "Dateiname".to_owned(),
        ("filename", Some("es")) => "nombre de archivo".to_owned(),
        ("filename", Some("eo")) => "dosiernomo".to_owned(),
        ("filename", Some("fi")) => "tiedostonimi".to_owned(),
        ("filename", Some("fr")) => "nom du fichier".to_owned(),
        ("filename", Some("it")) => "nome file".to_owned(),
        ("filename", Some("pt_PT" | "pt_BR")) => "nome do arquivo".to_owned(),
        ("filename", Some("tr")) => "dosya adi".to_owned(),
        ("filename", Some("ru")) => "imia faila".to_owned(),
        ("filename", Some("zh_CN")) => "wenjian mingcheng".to_owned(),
        ("filename", Some("ko")) => "pail ireum".to_owned(),
        ("filesize", Some("de")) => "Dateigroesse".to_owned(),
        ("filesize", Some("es")) => "tamano del archivo".to_owned(),
        ("filesize", Some("eo")) => "dosiergrando".to_owned(),
        ("filesize", Some("fi")) => "tiedostokoko".to_owned(),
        ("filesize", Some("fr")) => "taille du fichier".to_owned(),
        ("filesize", Some("it")) => "dimensione del file".to_owned(),
        ("filesize", Some("pt_PT" | "pt_BR")) => "tamanho do arquivo".to_owned(),
        ("filesize", Some("tr")) => "dosya boyutu".to_owned(),
        ("filesize", Some("ru")) => "razmer faila".to_owned(),
        ("filesize", Some("zh_CN")) => "wenjian daxiao".to_owned(),
        ("filesize", Some("ko")) => "pail keugi".to_owned(),
        ("duration", Some("de")) => "Dauer".to_owned(),
        ("duration", Some("es")) => "duracion".to_owned(),
        ("duration", Some("eo")) => "dauro".to_owned(),
        ("duration", Some("fi")) => "kesto".to_owned(),
        ("duration", Some("fr")) => "duree".to_owned(),
        ("duration", Some("it")) => "durata".to_owned(),
        ("duration", Some("pt_PT" | "pt_BR")) => "duracao".to_owned(),
        ("duration", Some("tr")) => "sure".to_owned(),
        ("duration", Some("ru")) => "dlitelnost".to_owned(),
        ("duration", Some("zh_CN")) => "shichang".to_owned(),
        ("duration", Some("ko")) => "gigan".to_owned(),
        _ => match token {
            "filename" => "filename".to_owned(),
            "filesize" => "filesize".to_owned(),
            "duration" => "duration".to_owned(),
            _ => token.to_owned(),
        },
    }
}

pub fn localized_file_difference_summary_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    summary
        .split(", ")
        .map(|token| localized_file_difference_token_legacy_compatible(token, language))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn localized_file_difference_notification_line_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    format!(
        "{}: {}",
        localized_file_differences_prefix_legacy_compatible(language),
        localized_file_difference_summary_legacy_compatible(summary, language)
    )
}

pub fn next_file_difference_notification_summary_legacy_compatible(
    state: &mut FileDifferenceNotificationState,
    summary: Option<FileDifferenceSummary>,
) -> Option<String> {
    let summary = summary.and_then(format_file_difference_summary);

    match summary {
        Some(summary) => {
            if state.last_summary.as_deref() != Some(summary.as_str()) {
                state.last_summary = Some(summary.clone());
                Some(summary)
            } else {
                state.last_summary = Some(summary);
                None
            }
        }
        None => {
            state.last_summary = None;
            None
        }
    }
}
