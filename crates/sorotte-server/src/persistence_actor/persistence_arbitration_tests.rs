use super::{
    ServerPersistenceEffect,
    persistence_arbitration::{
        RoomEffectEnqueueDisposition, RoomPersistenceArbitration, room_effect_key_and_version,
    },
};

fn save_effect(room_name: &str, version: u64) -> ServerPersistenceEffect {
    ServerPersistenceEffect::SaveRoom {
        room_name: room_name.to_owned(),
        files: vec![format!("{room_name}-{version}.mkv")],
        playlist_index: Some(version as i64),
        position: version as f64 + 0.25,
        last_activity_at_seconds: version as f64 + 0.5,
        owner_bucket: Some(format!("owner-{room_name}")),
        created_at_seconds: version as f64 + 0.75,
        version,
    }
}

fn delete_effect(room_name: &str, version: u64) -> ServerPersistenceEffect {
    ServerPersistenceEffect::DeleteRoom {
        room_name: room_name.to_owned(),
        version,
    }
}

fn stats_effect() -> ServerPersistenceEffect {
    ServerPersistenceEffect::RecordStatsSnapshot {
        snapshot_time: 41,
        versions: vec!["1.7.5".to_owned()],
    }
}

#[test]
fn effect_identity_preserves_room_kind_name_and_version() {
    assert_eq!(
        room_effect_key_and_version(&save_effect("alpha", 41)),
        Some(("alpha", 41))
    );
    assert_eq!(
        room_effect_key_and_version(&delete_effect("beta", 9)),
        Some(("beta", 9))
    );
    assert_eq!(room_effect_key_and_version(&stats_effect()), None);
}

#[test]
fn enqueue_rejects_non_room_zero_equal_and_older_effects() {
    let mut arbitration = RoomPersistenceArbitration::default();

    assert_eq!(
        arbitration.enqueue(stats_effect()),
        RoomEffectEnqueueDisposition::NotRoomEffect
    );
    assert_eq!(
        arbitration.enqueue(save_effect("alpha", 0)),
        RoomEffectEnqueueDisposition::IgnoredStale
    );
    assert!(arbitration.desired_effects().is_empty());

    let first = save_effect("alpha", 2);
    assert_eq!(
        arbitration.enqueue(first.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert_eq!(
        arbitration.enqueue(delete_effect("alpha", 2)),
        RoomEffectEnqueueDisposition::IgnoredStale
    );
    assert_eq!(
        arbitration.enqueue(delete_effect("alpha", 1)),
        RoomEffectEnqueueDisposition::IgnoredStale
    );
    assert_eq!(arbitration.desired_effects(), vec![first]);
}

#[test]
fn newer_effects_coalesce_independently_and_snapshot_in_room_order() {
    let mut arbitration = RoomPersistenceArbitration::default();
    let alpha_v1 = save_effect("alpha", 1);
    let alpha_v3 = delete_effect("alpha", 3);
    let beta_v2 = save_effect("beta", 2);

    assert_eq!(
        arbitration.enqueue(beta_v2.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert_eq!(
        arbitration.enqueue(alpha_v1.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert_eq!(
        arbitration.enqueue(alpha_v3.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );

    assert_eq!(
        arbitration.desired_effects(),
        vec![alpha_v3.clone(), beta_v2.clone()]
    );
    assert!(!arbitration.is_effect_current(&alpha_v1));
    assert!(arbitration.is_effect_current(&alpha_v3));
    assert!(arbitration.is_effect_current(&beta_v2));
    assert!(!arbitration.is_effect_current(&stats_effect()));
}

#[test]
fn effect_currency_requires_room_version_and_retained_desire() {
    let mut arbitration = RoomPersistenceArbitration::default();
    let current = save_effect("alpha", 5);
    assert_eq!(
        arbitration.enqueue(current.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );

    assert!(arbitration.is_effect_current(&current));
    assert!(!arbitration.is_effect_current(&save_effect("alpha", 4)));
    assert!(!arbitration.is_effect_current(&save_effect("alpha", 6)));
    assert!(!arbitration.is_effect_current(&save_effect("beta", 5)));
    assert!(arbitration.is_version_current("alpha", 5));
    assert!(!arbitration.is_version_current("alpha", 4));
    assert!(!arbitration.is_version_current("beta", 5));

    arbitration.mark_applied("alpha", 5, false);
    assert!(arbitration.is_version_current("alpha", 5));
    assert!(!arbitration.is_effect_current(&current));
    assert!(arbitration.desired_effects().is_empty());
}

#[test]
fn apply_completion_mutates_only_the_current_generation() {
    let mut arbitration = RoomPersistenceArbitration::default();
    let save_v3 = save_effect("alpha", 3);
    assert_eq!(
        arbitration.enqueue(save_v3.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );

    arbitration.mark_applied("alpha", 2, true);
    assert_eq!(arbitration.desired_effects(), vec![save_v3.clone()]);
    assert!(arbitration.is_version_current("alpha", 3));

    arbitration.mark_applied("alpha", 3, false);
    assert!(arbitration.is_settled());
    assert!(arbitration.is_version_current("alpha", 3));
    assert_eq!(
        arbitration.enqueue(delete_effect("alpha", 3)),
        RoomEffectEnqueueDisposition::IgnoredStale
    );

    let delete_v4 = delete_effect("alpha", 4);
    assert_eq!(
        arbitration.enqueue(delete_v4.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    arbitration.mark_applied("alpha", 3, true);
    assert_eq!(arbitration.desired_effects(), vec![delete_v4]);
    arbitration.mark_applied("alpha", 4, true);
    assert!(arbitration.is_settled());
    assert!(!arbitration.is_version_current("alpha", 4));
}

#[test]
fn failure_is_retained_until_current_success_or_newer_replacement() {
    let mut arbitration = RoomPersistenceArbitration::default();
    let save_v1 = save_effect("alpha", 1);
    assert_eq!(
        arbitration.enqueue(save_v1.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    arbitration.mark_applied("alpha", 1, false);
    assert!(arbitration.is_settled());

    arbitration.mark_failed("alpha", 0);
    assert!(arbitration.is_settled());
    arbitration.mark_failed("alpha", 1);
    assert!(!arbitration.is_settled());
    assert!(arbitration.desired_effects().is_empty());
    assert_eq!(
        arbitration.enqueue(delete_effect("alpha", 1)),
        RoomEffectEnqueueDisposition::IgnoredStale
    );
    assert!(!arbitration.is_settled());

    let save_v2 = save_effect("alpha", 2);
    assert_eq!(
        arbitration.enqueue(save_v2.clone()),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert_eq!(arbitration.desired_effects(), vec![save_v2]);
    arbitration.mark_applied("alpha", 2, false);
    assert!(arbitration.is_settled());
    arbitration.mark_failed("alpha", 1);
    assert!(arbitration.is_settled());
}

#[test]
fn recovery_requires_an_apply_and_every_room_to_be_settled() {
    let mut arbitration = RoomPersistenceArbitration::default();
    assert!(arbitration.is_settled());
    assert!(!arbitration.should_report_recovery(false));
    assert!(arbitration.should_report_recovery(true));

    assert_eq!(
        arbitration.enqueue(save_effect("alpha", 1)),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert_eq!(
        arbitration.enqueue(save_effect("beta", 1)),
        RoomEffectEnqueueDisposition::Accepted
    );
    assert!(!arbitration.is_settled());
    assert!(!arbitration.should_report_recovery(false));
    assert!(!arbitration.should_report_recovery(true));

    arbitration.mark_applied("alpha", 1, false);
    assert!(!arbitration.should_report_recovery(true));
    arbitration.mark_applied("beta", 1, false);
    assert!(arbitration.should_report_recovery(true));

    arbitration.mark_failed("alpha", 1);
    assert!(!arbitration.is_settled());
    assert!(!arbitration.should_report_recovery(true));
    assert_eq!(
        arbitration.enqueue(save_effect("alpha", 2)),
        RoomEffectEnqueueDisposition::Accepted
    );
    arbitration.mark_applied("alpha", 2, false);
    assert!(arbitration.should_report_recovery(true));
}
