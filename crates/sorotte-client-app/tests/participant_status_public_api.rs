use sorotte_client_app::app_boundary::participant_status::{
    ParticipantStatusPresentation, ParticipantStatusReportPresentation,
};
use sorotte_client_core::ClientParticipantStatusView;
use sorotte_protocol::{ParticipantStatusAvailability, ParticipantStatusView};

#[test]
fn downstream_status_presentation_uses_the_sanitizing_constructor_and_wildcard_match() {
    let view = ClientParticipantStatusView::from_wire(ParticipantStatusView::new(
        ParticipantStatusAvailability::Unavailable,
    ));
    let presentation = ParticipantStatusPresentation::Report(
        ParticipantStatusReportPresentation::from_client_view(view, false),
    );

    let category = match presentation {
        ParticipantStatusPresentation::Unavailable => "unavailable",
        ParticipantStatusPresentation::LegacyClient => "legacy",
        ParticipantStatusPresentation::WaitingForFirstReport => "waiting",
        ParticipantStatusPresentation::Report(_) => "report",
        _ => "future",
    };
    assert_eq!(category, "report");
}
