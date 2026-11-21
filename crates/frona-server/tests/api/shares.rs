use axum::http::StatusCode;
use tower::ServiceExt;

use super::*;

async fn status_of(state: &AppState, req: axum::http::Request<axum::body::Body>) -> StatusCode {
    build_app(state.clone())
        .oneshot(req)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_shared_agent_is_visible_and_usable_but_not_editable_by_the_recipient() {
    let (state, _tmp) = test_app_state().await;
    let (owner, _) = register_user(&state, "owner1", "owner1@example.com", "password123").await;
    let (friend, _) = register_user(&state, "friend1", "friend1@example.com", "password123").await;
    let (stranger, _) =
        register_user(&state, "stranger1", "stranger1@example.com", "password123").await;

    let agent = create_agent(&state, &owner, "Shared Helper").await;
    let agent_id = agent["id"].as_str().unwrap().to_string();

    // Before sharing, the friend can neither see nor open it.
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/agents/{agent_id}"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );

    // The owner shares by handle.
    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/agents/{agent_id}/shares"),
            &owner,
            serde_json::json!({ "recipient": "friend1" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let shares = body_json(resp).await;
    assert_eq!(shares.as_array().unwrap().len(), 1);
    assert_eq!(shares[0]["recipient_handle"], "friend1");
    assert_eq!(shares[0]["delegate_credentials"], false);

    // The friend now sees it, marked shared and attributed to the owner.
    let resp = build_app(state.clone())
        .oneshot(auth_get("/api/agents", &friend))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await;
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == agent_id.as_str())
        .expect("shared agent is listed for the recipient");
    assert_eq!(entry["is_shared"], true);
    assert_eq!(entry["shared_by"], "owner1");

    let resp = build_app(state.clone())
        .oneshot(auth_get(&format!("/api/agents/{agent_id}"), &friend))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Editing and managing shares stay owner-only; strangers see nothing.
    assert_eq!(
        status_of(
            &state,
            auth_put_json(
                &format!("/api/agents/{agent_id}"),
                &friend,
                serde_json::json!({ "name": "Hijacked" })
            )
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/agents/{agent_id}/shares"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/agents/{agent_id}"), &stranger)
        )
        .await,
        StatusCode::FORBIDDEN
    );

    // Unsharing takes access away again.
    let recipient_id = shares[0]["recipient_id"].as_str().unwrap().to_string();
    assert_eq!(
        status_of(
            &state,
            auth_delete(
                &format!("/api/agents/{agent_id}/shares/{recipient_id}"),
                &owner
            )
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/agents/{agent_id}"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_shared_chat_is_readable_but_not_writable_by_the_recipient() {
    let (state, _tmp) = test_app_state().await;
    let (owner, _) = register_user(&state, "owner2", "owner2@example.com", "password123").await;
    let (friend, _) = register_user(&state, "friend2", "friend2@example.com", "password123").await;

    let agent = create_agent(&state, &owner, "Chat Agent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            "/api/chats",
            &owner,
            serde_json::json!({ "agent_id": agent_id, "title": "Private plans" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let chat_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    assert_eq!(
        status_of(&state, auth_get(&format!("/api/chats/{chat_id}"), &friend)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/chats/{chat_id}/messages"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );

    let resp = build_app(state.clone())
        .oneshot(auth_post_json(
            &format!("/api/chats/{chat_id}/shares"),
            &owner,
            serde_json::json!({ "recipient": "friend2@example.com" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read access works and is marked as shared.
    let resp = build_app(state.clone())
        .oneshot(auth_get(&format!("/api/chats/{chat_id}"), &friend))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let chat = body_json(resp).await;
    assert_eq!(chat["is_shared"], true);
    assert_eq!(chat["shared_by"], "owner2");
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/chats/{chat_id}/messages"), &friend)
        )
        .await,
        StatusCode::OK
    );

    // The chat appears in the recipient's list, merged after their own.
    let resp = build_app(state.clone())
        .oneshot(auth_get("/api/chats", &friend))
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == chat_id.as_str() && c["is_shared"] == true)
    );

    // Mutations stay owner-only.
    assert_eq!(
        status_of(
            &state,
            auth_delete(&format!("/api/chats/{chat_id}"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            &state,
            auth_get(&format!("/api/chats/{chat_id}/shares"), &friend)
        )
        .await,
        StatusCode::FORBIDDEN
    );
}
