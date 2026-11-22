use axum::body::Body;
use axum::http::{Request, StatusCode};
use frona::notification::models::{NotificationData, NotificationLevel};
use tower::ServiceExt;

use super::*;

async fn seed_notification(state: &AppState, user_id: &str, title: &str) {
    state
        .notification_service
        .create(
            user_id,
            NotificationData::System {},
            NotificationLevel::Info,
            title.to_string(),
            "test body".to_string(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn list_notifications_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(
        &state,
        "notif-empty",
        "notifempty@example.com",
        "password123",
    )
    .await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 0);
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn list_notifications_returns_created() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-list", "notiflist@example.com", "password123").await;

    seed_notification(&state, &user_id, "First").await;
    seed_notification(&state, &user_id, "Second").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 2);
    assert_eq!(json["unread_count"], 2);
}

#[tokio::test]
async fn list_notifications_isolates_users() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, user_id_a) =
        register_user(&state, "notif-a", "notifa@example.com", "password123").await;
    let (token_b, user_id_b) =
        register_user(&state, "notif-b", "notifb@example.com", "password123").await;

    seed_notification(&state, &user_id_a, "For A").await;
    seed_notification(&state, &user_id_b, "For B").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/notifications", &token_a))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(json["notifications"][0]["title"], "For A");

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token_b))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(json["notifications"][0]["title"], "For B");
}

#[tokio::test]
async fn list_notifications_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mark_read_succeeds() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-read", "notifread@example.com", "password123").await;

    seed_notification(&state, &user_id, "To read").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let notif_id = json["notifications"][0]["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/notifications/{notif_id}/read"),
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn mark_read_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/notifications/fake-id/read")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mark_all_read_succeeds() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "notif-all", "notifall@example.com", "password123").await;

    seed_notification(&state, &user_id, "One").await;
    seed_notification(&state, &user_id, "Two").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/notifications/read-all",
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/notifications", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["unread_count"], 0);
}

#[tokio::test]
async fn mark_all_read_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/notifications/read-all")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_agent_reply_notifies_the_user_with_a_preview_and_a_link_to_the_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) = register_user(
        &state,
        "notif-reply",
        "notifreply@example.com",
        "password123",
    )
    .await;
    let agent = create_agent(&state, &token, "Scout").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let chat = create_chat(&state, &token, &agent_id, Some("Trip")).await;
    let chat_id = chat["id"].as_str().unwrap().to_string();

    let long_reply = "x".repeat(500);
    state
        .chat_service
        .save_agent_message(&user_id, None, &chat_id, &agent_id, long_reply, None)
        .await
        .unwrap();

    let list = state.notification_service.list(&user_id, 10).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Scout");
    assert_eq!(
        list[0].body.chars().count(),
        201,
        "200 chars plus an ellipsis"
    );
    assert!(list[0].body.ends_with('…'));
    match &list[0].data {
        NotificationData::Agent {
            agent_id: a,
            chat_id: c,
        } => {
            assert_eq!(a, &agent_id);
            assert_eq!(c, &chat_id);
        }
        other => panic!("expected an agent notification, got {other:?}"),
    }
}

#[tokio::test]
async fn system_messages_and_empty_replies_do_not_notify() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) = register_user(
        &state,
        "notif-quiet",
        "notifquiet@example.com",
        "password123",
    )
    .await;
    let agent = create_agent(&state, &token, "Scout").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let chat = create_chat(&state, &token, &agent_id, None).await;
    let chat_id = chat["id"].as_str().unwrap().to_string();

    state
        .chat_service
        .save_system_message(&user_id, None, &chat_id, "joined".to_string())
        .await
        .unwrap();
    state
        .chat_service
        .save_agent_message(&user_id, None, &chat_id, &agent_id, String::new(), None)
        .await
        .unwrap();

    let list = state.notification_service.list(&user_id, 10).await.unwrap();
    assert!(list.is_empty());
}
