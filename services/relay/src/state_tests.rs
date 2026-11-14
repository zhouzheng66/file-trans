use super::*;

fn service() -> RelayState {
    RelayState::new(
        RelayLimits::default(),
        Identity::generate().unwrap(),
        CancellationToken::new(),
    )
    .unwrap()
}

#[test]
fn expires_invites_and_releases_pending_session_capacity() {
    let state = service();
    let (source, _source_rx) = mpsc::channel(8);
    let (target, _target_rx) = mpsc::channel(8);
    state.register("source", "a".into(), source).unwrap();
    state.register("target", "b".into(), target).unwrap();
    let reply = state
        .dispatch(
            "target",
            ClientMessage::Invite {
                request_id: Uuid::new_v4().to_string(),
            },
        )
        .unwrap()
        .unwrap();
    let ServerMessage::Invite { code, .. } = reply else {
        panic!("expected invite")
    };
    let reply = state
        .dispatch(
            "source",
            ClientMessage::Open {
                request_id: Uuid::new_v4().to_string(),
                target: Target::Invite(code.clone()),
            },
        )
        .unwrap()
        .unwrap();
    let ServerMessage::Opened { session_id, .. } = reply else {
        panic!("expected session")
    };
    assert_eq!(
        state.slots.available_permits(),
        state.limits.max_sessions - 1
    );
    {
        let mut tables = state.tables.lock().unwrap();
        tables.invites.get_mut(&code).unwrap().1 = 0;
        tables.sessions.get_mut(&session_id).unwrap().deadline = Instant::now();
    }
    assert!(state
        .dispatch(
            "source",
            ClientMessage::Open {
                request_id: Uuid::new_v4().to_string(),
                target: Target::Invite(code)
            }
        )
        .is_err());
    state.prune();
    assert_eq!(state.slots.available_permits(), state.limits.max_sessions);
    assert!(state.tables.lock().unwrap().usage.is_empty());
}

#[test]
fn only_session_participants_can_cancel_and_disconnect_cleans_up() {
    let state = service();
    let (source, _source_rx) = mpsc::channel(8);
    let (target, _target_rx) = mpsc::channel(8);
    state.register("source", "a".into(), source).unwrap();
    state.register("target", "b".into(), target).unwrap();
    let reply = state
        .dispatch(
            "source",
            ClientMessage::Open {
                request_id: Uuid::new_v4().to_string(),
                target: Target::Peer("target".into()),
            },
        )
        .unwrap()
        .unwrap();
    let ServerMessage::Opened { session_id, .. } = reply else {
        panic!("expected session")
    };
    state
        .dispatch(
            "stranger",
            ClientMessage::Reject {
                session_id: session_id.clone(),
            },
        )
        .unwrap();
    assert!(state
        .tables
        .lock()
        .unwrap()
        .sessions
        .contains_key(&session_id));
    state.disconnect("source", "obsolete-generation");
    assert!(state.tables.lock().unwrap().controls.contains_key("source"));
    state.disconnect("source", "a");
    let tables = state.tables.lock().unwrap();
    assert!(!tables.controls.contains_key("source"));
    assert!(tables.sessions.is_empty());
    assert!(tables.usage.is_empty());
}
