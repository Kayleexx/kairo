#![allow(clippy::expect_used)]

use kairo_control::{
    LiveEdgeMetrics, LiveEdgeParticipant, LiveEdgeSession, LiveEdgeState, RunOutput,
};

#[test]
fn producer_completion_may_arrive_after_quic_handshake_before_streaming_marker() {
    let mut session = LiveEdgeSession::pending(
        "session".into(),
        "run".into(),
        "edge".into(),
        1,
        0,
        "producer".into(),
        1,
        "consumer".into(),
    );
    session
        .transition(
            LiveEdgeParticipant::Producer,
            "producer",
            1,
            LiveEdgeState::Assigned,
            None,
        )
        .expect("assigned");
    session
        .transition(
            LiveEdgeParticipant::Producer,
            "producer",
            1,
            LiveEdgeState::Ready,
            Some("127.0.0.1:1".into()),
        )
        .expect("ready");
    session
        .complete(
            LiveEdgeParticipant::Producer,
            "producer",
            1,
            None,
            Some(LiveEdgeMetrics {
                bytes: 3,
                duration_us: 1,
                peak_buffered_bytes: 3,
            }),
        )
        .expect("producer completion");
    assert!(matches!(session.state, LiveEdgeState::Ready));
    assert_eq!(
        session
            .observation
            .as_ref()
            .and_then(|value| value.producer.as_ref())
            .map(|metrics| metrics.bytes),
        Some(3)
    );
    session
        .transition(
            LiveEdgeParticipant::Consumer,
            "consumer",
            1,
            LiveEdgeState::Streaming,
            None,
        )
        .expect("streaming");
    session
        .complete(
            LiveEdgeParticipant::Consumer,
            "consumer",
            1,
            Some(RunOutput::Scalar(3)),
            None,
        )
        .expect("consumer completion");
    assert!(matches!(session.state, LiveEdgeState::Completed));
}
