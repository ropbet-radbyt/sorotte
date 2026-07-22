use sorotte_plex::PlexError;

fn legacy_error_category(error: &PlexError) -> &'static str {
    match error {
        PlexError::Http(_) => "http",
        PlexError::Json(_) => "json",
        PlexError::Io(_) => "io",
        PlexError::InvalidResponse(_) => "response",
        PlexError::MissingServer => "server",
        PlexError::MissingToken => "token",
    }
}

#[test]
fn legacy_plex_error_exhaustive_match_remains_source_compatible() {
    let error = PlexError::InvalidResponse("fixture".to_owned());
    assert_eq!(legacy_error_category(&error), "response");
}
