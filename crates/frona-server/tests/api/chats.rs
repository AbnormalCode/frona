use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn create_chat_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "chatuser", "chatuser@example.com", "password123").await;
    let agent = create_agent(&state, &token, "ChatAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let chat = create_chat(&state, &token, agent_id, Some("Hello")).await;
    assert!(chat["id"].is_string());
    assert_eq!(chat["agent_id"], agent_id);
    assert_eq!(chat["title"], "Hello");
}

#[tokio::test]
async fn chat_metadata_round_trip_and_partial_patch() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "cmd", "cmd@example.com", "password123").await;
    let agent = create_agent(&state, &token, "MetaAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/chats",
            &token,
            serde_json::json!({
                "agent_id": agent_id,
                "metadata": {"channel:external_id": "dm:42", "tag": "hi"},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let chat = body_json(resp).await;
    assert_eq!(chat["metadata"]["channel:external_id"], "dm:42");
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/chats/{id}"),
            &token,
            serde_json::json!({
                "metadata": {"channel:external_id": "dm:99", "tag": null},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metadata"]["channel:external_id"], "dm:99");
    assert!(json["metadata"].get("tag").is_none());
}

#[tokio::test]
async fn create_chat_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chats")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"agent_id": "fake"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_chats_returns_only_own() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) = register_user(&state, "chat-a", "chata@example.com", "password123").await;
    let (token_b, _) = register_user(&state, "chat-b", "chatb@example.com", "password123").await;

    let agent_a = create_agent(&state, &token_a, "AgA").await;
    let agent_b = create_agent(&state, &token_b, "AgB").await;

    create_chat(&state, &token_a, agent_a["id"].as_str().unwrap(), None).await;
    create_chat(&state, &token_b, agent_b["id"].as_str().unwrap(), None).await;

    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/chats", &token_a)).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn get_chat_by_id() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "getchat", "getchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "GC").await;
    let chat = create_chat(
        &state,
        &token,
        agent["id"].as_str().unwrap(),
        Some("MyChat"),
    )
    .await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["title"], "MyChat");
}

#[tokio::test]
async fn get_chat_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) = register_user(&state, "chatown", "chatown@example.com", "password123").await;
    let (token_b, _) = register_user(&state, "chatoth", "chatoth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "CO").await;
    let chat = create_chat(&state, &token_a, agent["id"].as_str().unwrap(), None).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token_b))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Expected 404 or 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn update_chat_title() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "upchat", "upchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "UC").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), Some("Old")).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/chats/{id}"),
            &token,
            serde_json::json!({"title": "New Title"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["title"], "New Title");
}

#[tokio::test]
async fn delete_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "delchat", "delchat@example.com", "password123").await;
    let agent = create_agent(&state, &token, "DC").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), None).await;
    let id = chat["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(&format!("/api/chats/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn archive_and_unarchive_chat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "archuser", "archuser@example.com", "password123").await;
    let agent = create_agent(&state, &token, "AR").await;
    let chat = create_chat(&state, &token, agent["id"].as_str().unwrap(), Some("Arch")).await;
    let id = chat["id"].as_str().unwrap();

    // Archive
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/chats/{id}/archive"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["archived_at"].is_string());

    // Should not appear in main list
    let app = build_app(state.clone());
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let list = body_json(resp).await;
    assert!(
        list.as_array().unwrap().iter().all(|c| c["id"] != id),
        "Archived chat should not appear in main list"
    );

    // Should appear in archived list
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_get("/api/chats/archived", &token))
        .await
        .unwrap();
    let archived = body_json(resp).await;
    assert!(archived.as_array().unwrap().iter().any(|c| c["id"] == id));

    // Unarchive
    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/chats/{id}/unarchive"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["archived_at"].is_null());

    // Should be back in main list
    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/chats", &token)).await.unwrap();
    let list = body_json(resp).await;
    assert!(list.as_array().unwrap().iter().any(|c| c["id"] == id));
}

#[tokio::test]
async fn delegations_list_the_sub_tasks_a_chat_spawned_with_their_agent_and_status() {
    use frona::agent::task::models::CreateTaskRequest;

    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "deleg-a", "delega@example.com", "password123").await;
    let parent = create_agent(&state, &token, "Planner").await;
    let child = create_agent(&state, &token, "Scout").await;
    let parent_id = parent["id"].as_str().unwrap().to_string();
    let child_id = child["id"].as_str().unwrap().to_string();
    let chat = create_chat(&state, &token, &parent_id, Some("Plan")).await;
    let chat_id = chat["id"].as_str().unwrap().to_string();

    let task = state
        .task_service
        .create(
            &user_id,
            CreateTaskRequest {
                agent_id: child_id.clone(),
                space_id: None,
                chat_id: None,
                title: "Look it up".into(),
                description: None,
                source_agent_id: Some(parent_id.clone()),
                source_chat_id: Some(chat_id.clone()),
                resume_parent: None,
                run_at: None,
                quarantined: false,
                result_schema: None,
                result_description: None,
            },
        )
        .await
        .unwrap();

    let resp = build_app(state.clone())
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/delegations"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let list = json.as_array().unwrap();
    assert_eq!(list.len(), 1, "{json}");
    assert_eq!(list[0]["task_id"], task.id.as_str());
    assert_eq!(list[0]["agent_id"], child_id.as_str());
    assert_eq!(list[0]["agent_name"], "Scout");
    assert_eq!(list[0]["status"], "pending");

    // A paused-for-approval delegate is reported as such.
    state
        .task_service
        .mark_in_progress(&task.id, None)
        .await
        .unwrap();
    state
        .task_service
        .mark_waiting_approval(&task.id)
        .await
        .unwrap();
    let json = body_json(
        build_app(state.clone())
            .oneshot(auth_get(
                &format!("/api/chats/{chat_id}/delegations"),
                &token,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(json[0]["status"], "waitingapproval");

    // Someone else's chat is not theirs to inspect.
    let (other, _) = register_user(&state, "deleg-b", "delegb@example.com", "password123").await;
    let resp = build_app(state)
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/delegations"),
            &other,
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "{}",
        resp.status()
    );
}
