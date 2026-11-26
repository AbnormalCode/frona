//! A turn that was superseded by a message sent while it ran must not announce
//! a Stop: the successor turn owns the UI's running state, and a "cancelled"
//! broadcast for the old one would reset it mid-interrupt.

use frona::chat::broadcast::BroadcastEventKind;
use frona::chat::message::models::{Message, MessageRole, MessageStatus};
use frona::core::repository::Repository;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::tool_loop::InferenceEventKind;
use surrealdb::{
    Surreal,
    engine::local::{Db, Mem},
};

mod helpers;

async fn fixture() -> (Surreal<Db>, helpers::app_state::Fixture) {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    let mut chat = helpers::mock_context().chat.unwrap();
    chat.id = "chat".into();
    chat.user_id = "user".into();
    chat.agent_id = "agent".into();
    SurrealRepo::new(db.clone()).create(&chat).await.unwrap();
    let state = helpers::app_state::build(&db).await;
    (db, state)
}

async fn executing_message(db: &Surreal<Db>) -> Message {
    let repo: SurrealRepo<Message> = SurrealRepo::new(db.clone());
    repo.create(
        &Message::builder("chat", MessageRole::Agent, String::new())
            .agent_id("agent".to_string())
            .status(MessageStatus::Executing)
            .build(),
    )
    .await
    .unwrap()
}

fn cancelled_broadcasts(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<frona::chat::broadcast::BroadcastEvent>,
) -> usize {
    let mut cancelled = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.kind,
            BroadcastEventKind::Inference(InferenceEventKind::Cancelled { .. })
        ) {
            cancelled += 1;
        }
    }
    cancelled
}

#[tokio::test]
async fn a_stopped_turn_announces_the_cancellation() {
    let (db, app) = fixture().await;
    let message = executing_message(&db).await;
    let mut events = app.state.broadcast_service.subscribe_raw();

    let cancelled = app
        .state
        .chat_service
        .cancel_agent_message_notifying(message, true)
        .await
        .unwrap();

    assert_eq!(cancelled.status, Some(MessageStatus::Cancelled));
    assert_eq!(cancelled_broadcasts(&mut events), 1);
}

#[tokio::test]
async fn a_superseded_turn_is_marked_cancelled_but_stays_quiet() {
    let (db, app) = fixture().await;
    let message = executing_message(&db).await;
    let mut events = app.state.broadcast_service.subscribe_raw();

    let cancelled = app
        .state
        .chat_service
        .cancel_agent_message_notifying(message, false)
        .await
        .unwrap();

    assert_eq!(cancelled.status, Some(MessageStatus::Cancelled));
    assert_eq!(cancelled_broadcasts(&mut events), 0);
}
