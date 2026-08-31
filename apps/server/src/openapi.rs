use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy)]
enum Security {
    Public,
    Bearer,
    Admin,
    WebSocketTicket,
}

#[derive(Debug, Clone, Copy)]
struct Operation {
    path: &'static str,
    method: &'static str,
    summary: &'static str,
    security: Security,
}

macro_rules! operation {
    ($method:literal, $path:literal, $summary:literal, $security:ident) => {
        Operation {
            path: $path,
            method: $method,
            summary: $summary,
            security: Security::$security,
        }
    };
}

const OPERATIONS: &[Operation] = &[
    operation!("get", "/openapi.json", "Read this OpenAPI document", Public),
    operation!("post", "/auth/register", "Register an account", Public),
    operation!("post", "/auth/login", "Create a session", Public),
    operation!("post", "/auth/qr-login", "Start QR-code login", Public),
    operation!(
        "post",
        "/auth/qr-login/{challenge_id}/poll",
        "Poll QR-code login",
        Public
    ),
    operation!("post", "/auth/refresh", "Rotate a refresh token", Public),
    operation!("post", "/auth/logout", "Revoke a refresh token", Public),
    operation!(
        "post",
        "/auth/password-reset/request",
        "Request a password reset",
        Public
    ),
    operation!(
        "post",
        "/auth/password-reset/confirm",
        "Confirm a password reset",
        Public
    ),
    operation!(
        "post",
        "/auth/change-password",
        "Change the current password",
        Bearer
    ),
    operation!("get", "/devices", "List signed-in devices", Bearer),
    operation!("delete", "/devices/{device_id}", "Revoke a device", Bearer),
    operation!("get", "/me", "Read the current profile", Bearer),
    operation!("patch", "/me", "Update the current profile", Bearer),
    operation!("delete", "/me", "Delete and anonymize the account", Bearer),
    operation!("get", "/me/second-factor", "Read two-factor status", Bearer),
    operation!(
        "post",
        "/me/second-factor",
        "Begin two-factor setup",
        Bearer
    ),
    operation!(
        "delete",
        "/me/second-factor",
        "Disable two-factor authentication",
        Bearer
    ),
    operation!(
        "post",
        "/me/second-factor/enable",
        "Enable two-factor authentication",
        Bearer
    ),
    operation!(
        "post",
        "/me/second-factor/recovery-codes",
        "Regenerate recovery codes",
        Bearer
    ),
    operation!(
        "post",
        "/auth/qr-login/{challenge_id}/approve",
        "Approve QR-code login",
        Bearer
    ),
    operation!("get", "/me/privacy", "Read profile privacy", Bearer),
    operation!("patch", "/me/privacy", "Update profile privacy", Bearer),
    operation!("get", "/me/export", "Export personal account data", Bearer),
    operation!("get", "/me/stickers", "List custom stickers", Bearer),
    operation!("post", "/me/stickers", "Create a custom sticker", Bearer),
    operation!(
        "delete",
        "/me/stickers/{sticker_id}",
        "Delete a custom sticker",
        Bearer
    ),
    operation!("get", "/bootstrap", "Load initial client state", Bearer),
    operation!(
        "get",
        "/users/search",
        "Find a user by exact username",
        Bearer
    ),
    operation!("get", "/friends", "List friends", Bearer),
    operation!("get", "/friends/settings", "List friend settings", Bearer),
    operation!(
        "patch",
        "/friends/{friend_id}",
        "Update friend settings",
        Bearer
    ),
    operation!("delete", "/friends/{friend_id}", "Delete a friend", Bearer),
    operation!("post", "/blocks/{user_id}", "Block a user", Bearer),
    operation!("delete", "/blocks/{user_id}", "Unblock a user", Bearer),
    operation!("post", "/reports/{user_id}", "Report a user", Bearer),
    operation!("get", "/friend-requests", "List friend requests", Bearer),
    operation!(
        "post",
        "/friend-requests",
        "Create a friend request",
        Bearer
    ),
    operation!(
        "patch",
        "/friend-requests/{request_id}",
        "Decide a friend request",
        Bearer
    ),
    operation!("get", "/conversations", "List conversations", Bearer),
    operation!(
        "post",
        "/conversations/read-all",
        "Mark all conversations read",
        Bearer
    ),
    operation!(
        "post",
        "/conversations/direct",
        "Create or find a direct conversation",
        Bearer
    ),
    operation!(
        "post",
        "/conversations/group",
        "Create a group conversation",
        Bearer
    ),
    operation!(
        "get",
        "/groups/{conversation_id}",
        "Read group settings",
        Bearer
    ),
    operation!(
        "patch",
        "/groups/{conversation_id}",
        "Update a group",
        Bearer
    ),
    operation!(
        "delete",
        "/groups/{conversation_id}",
        "Disband a group",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/members",
        "Add group members",
        Bearer
    ),
    operation!(
        "patch",
        "/groups/{conversation_id}/members/{member_id}",
        "Update a group member",
        Bearer
    ),
    operation!(
        "delete",
        "/groups/{conversation_id}/members/{member_id}",
        "Remove a group member",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/leave",
        "Leave a group",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/transfer",
        "Transfer group ownership",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/mute",
        "Set group-wide mute",
        Bearer
    ),
    operation!(
        "get",
        "/groups/{conversation_id}/announcements",
        "List group announcements",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/announcements",
        "Create a group announcement",
        Bearer
    ),
    operation!(
        "post",
        "/group-announcements/{announcement_id}/read",
        "Mark an announcement read",
        Bearer
    ),
    operation!(
        "get",
        "/groups/{conversation_id}/join-requests",
        "List group join requests",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/join-requests",
        "Request to join a group",
        Bearer
    ),
    operation!(
        "patch",
        "/group-join-requests/{request_id}",
        "Decide a group join request",
        Bearer
    ),
    operation!(
        "get",
        "/groups/{conversation_id}/polls",
        "List group polls",
        Bearer
    ),
    operation!(
        "post",
        "/groups/{conversation_id}/polls",
        "Create a group poll",
        Bearer
    ),
    operation!(
        "get",
        "/groups/{conversation_id}/files",
        "List group files",
        Bearer
    ),
    operation!(
        "post",
        "/polls/{poll_id}/vote",
        "Vote in a group poll",
        Bearer
    ),
    operation!(
        "get",
        "/conversations/{conversation_id}/settings",
        "Read conversation settings",
        Bearer
    ),
    operation!(
        "patch",
        "/conversations/{conversation_id}/settings",
        "Update conversation settings",
        Bearer
    ),
    operation!(
        "get",
        "/conversations/{conversation_id}/messages",
        "Page through messages",
        Bearer
    ),
    operation!(
        "post",
        "/conversations/{conversation_id}/messages",
        "Send an idempotent message",
        Bearer
    ),
    operation!("post", "/messages/forward", "Forward messages", Bearer),
    operation!(
        "get",
        "/messages/favorites",
        "List favorite messages",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/translate",
        "Translate a text message",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/transcribe",
        "Transcribe a voice message",
        Bearer
    ),
    operation!(
        "get",
        "/messages/{message_id}",
        "Read message details and receipts",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/delivery",
        "Acknowledge message delivery",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/recall",
        "Recall a message",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/reaction",
        "Set a message reaction",
        Bearer
    ),
    operation!(
        "post",
        "/messages/{message_id}/favorite",
        "Set message favorite state",
        Bearer
    ),
    operation!(
        "get",
        "/scheduled-messages",
        "List scheduled messages",
        Bearer
    ),
    operation!("post", "/scheduled-messages", "Schedule a message", Bearer),
    operation!(
        "delete",
        "/scheduled-messages/{schedule_id}",
        "Cancel a scheduled message",
        Bearer
    ),
    operation!(
        "post",
        "/conversations/{conversation_id}/read",
        "Advance the read cursor",
        Bearer
    ),
    operation!(
        "get",
        "/sync",
        "Pull events using a monotonic cursor",
        Bearer
    ),
    operation!(
        "post",
        "/uploads/authorize",
        "Authorize an attachment upload",
        Bearer
    ),
    operation!(
        "post",
        "/uploads/complete",
        "Complete and verify an attachment upload",
        Bearer
    ),
    operation!(
        "get",
        "/attachments/{attachment_id}/download",
        "Authorize an attachment download",
        Bearer
    ),
    operation!(
        "post",
        "/ws-ticket",
        "Create a short-lived WebSocket ticket",
        Bearer
    ),
    operation!(
        "get",
        "/ws",
        "Upgrade to the realtime WebSocket protocol",
        WebSocketTicket
    ),
    operation!(
        "post",
        "/admin/users/{user_id}/suspension",
        "Suspend or restore a user",
        Admin
    ),
    operation!(
        "post",
        "/admin/users/{user_id}/sessions/revoke",
        "Revoke every user session",
        Admin
    ),
    operation!(
        "get",
        "/admin/audit",
        "Read the administration audit trail",
        Admin
    ),
];

pub fn document() -> Value {
    let mut paths = Map::new();
    for operation in OPERATIONS {
        let path = paths
            .entry(operation.path.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        path.as_object_mut()
            .expect("path object")
            .insert(operation.method.to_owned(), operation_document(*operation));
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "I Am Rust API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Versioned REST and WebSocket contract for the I Am Rust desktop IM client."
        },
        "servers": [{ "url": "/api/v1" }],
        "tags": [
            { "name": "auth" }, { "name": "account" }, { "name": "social" },
            { "name": "conversations" }, { "name": "groups" }, { "name": "messages" },
            { "name": "media" }, { "name": "realtime" }, { "name": "admin" }
        ],
        "paths": paths,
        "components": components()
    })
}

fn operation_document(operation: Operation) -> Value {
    let success_status = if operation.path == "/ws" {
        "101"
    } else {
        "200"
    };
    let mut value = json!({
        "operationId": operation_id(operation),
        "summary": operation.summary,
        "tags": [tag(operation.path)],
        "parameters": parameters(operation.path),
        "responses": {
            (success_status): { "description": "Success" },
            "400": { "$ref": "#/components/responses/BadRequest" },
            "401": { "$ref": "#/components/responses/Unauthorized" },
            "403": { "$ref": "#/components/responses/Forbidden" },
            "404": { "$ref": "#/components/responses/NotFound" },
            "409": { "$ref": "#/components/responses/Conflict" },
            "429": { "$ref": "#/components/responses/RateLimited" },
            "500": { "$ref": "#/components/responses/InternalError" }
        }
    });
    let object = value.as_object_mut().expect("operation object");
    match operation.security {
        Security::Public => {}
        Security::Bearer => {
            object.insert("security".to_owned(), json!([{ "bearerAuth": [] }]));
        }
        Security::Admin => {
            object.insert("security".to_owned(), json!([{ "adminToken": [] }]));
        }
        Security::WebSocketTicket => {
            object.insert("security".to_owned(), json!([{ "websocketTicket": [] }]));
        }
    }
    if matches!(operation.method, "post" | "patch" | "delete") {
        object.insert(
            "requestBody".to_owned(),
            json!({
                "required": false,
                "content": {
                    "application/json": {
                        "schema": { "type": "object", "additionalProperties": true }
                    }
                }
            }),
        );
    }
    value
}

fn parameters(path: &str) -> Vec<Value> {
    let mut parameters = path
        .split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
        })
        .map(|name| {
            json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": { "type": "string", "format": "uuid" }
            })
        })
        .collect::<Vec<_>>();
    match path {
        "/users/search" => parameters.push(query_parameter("username", "string", true)),
        "/sync" => {
            parameters.push(query_parameter("after", "integer", true));
            parameters.push(query_parameter("limit", "integer", false));
        }
        "/conversations/{conversation_id}/messages" => {
            parameters.push(query_parameter("before", "integer", false));
            parameters.push(query_parameter("limit", "integer", false));
        }
        "/ws" => parameters.push(query_parameter("ticket", "string", true)),
        "/admin/audit" => parameters.push(query_parameter("limit", "integer", false)),
        _ => {}
    }
    parameters
}

fn query_parameter(name: &str, kind: &str, required: bool) -> Value {
    json!({
        "name": name,
        "in": "query",
        "required": required,
        "schema": { "type": kind }
    })
}

fn operation_id(operation: Operation) -> String {
    format!("{}_{}", operation.method, operation.path.trim_matches('/'))
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn tag(path: &str) -> &'static str {
    if path.starts_with("/auth") {
        "auth"
    } else if path.starts_with("/me") || path.starts_with("/devices") {
        "account"
    } else if path.starts_with("/friends")
        || path.starts_with("/blocks")
        || path.starts_with("/reports")
        || path.starts_with("/users")
    {
        "social"
    } else if path.starts_with("/groups")
        || path.starts_with("/group-")
        || path.starts_with("/polls")
    {
        "groups"
    } else if path.starts_with("/messages") || path.starts_with("/scheduled-messages") {
        "messages"
    } else if path.starts_with("/uploads") || path.starts_with("/attachments") {
        "media"
    } else if path.starts_with("/ws") || path == "/sync" {
        "realtime"
    } else if path.starts_with("/admin") {
        "admin"
    } else {
        "conversations"
    }
}

fn components() -> Value {
    let error_response = |description: &str| {
        json!({
            "description": description,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorResponse" } } }
        })
    };
    json!({
        "securitySchemes": {
            "bearerAuth": { "type": "http", "scheme": "bearer" },
            "adminToken": { "type": "apiKey", "in": "header", "name": "x-admin-token" },
            "websocketTicket": { "type": "apiKey", "in": "query", "name": "ticket" }
        },
        "responses": {
            "BadRequest": error_response("Malformed or invalid request"),
            "Unauthorized": error_response("Authentication failed"),
            "Forbidden": error_response("The authenticated user lacks access"),
            "NotFound": error_response("Resource not found"),
            "Conflict": error_response("Resource state conflict"),
            "RateLimited": error_response("Rate limit exceeded"),
            "InternalError": error_response("Unexpected server failure")
        },
        "schemas": {
            "ErrorResponse": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code", "message_key", "correlation_id", "retryable"],
                "properties": {
                    "code": { "type": "string", "example": "validation" },
                    "message_key": { "type": "string", "example": "error.validation" },
                    "field": { "type": ["string", "null"] },
                    "correlation_id": { "type": "string", "format": "uuid" },
                    "retryable": { "type": "boolean" }
                }
            },
            "WebSocketClientFrame": {
                "oneOf": [
                    { "example": { "type": "hello", "protocol_version": 1, "client_version": "0.1.0", "access_token": "", "last_cursor": 42 } },
                    { "example": { "type": "ping", "nonce": "018f0000-0000-7000-8000-000000000001" } },
                    { "example": { "type": "typing", "conversation_id": "018f0000-0000-7000-8000-000000000002", "active": true } }
                ]
            },
            "WebSocketServerFrame": {
                "oneOf": [
                    { "example": { "type": "welcome", "protocol_version": 1, "latest_cursor": 42 } },
                    { "example": { "type": "event", "event": { "id": "018f0000-0000-7000-8000-000000000003", "cursor": 43, "kind": "message_created", "payload": {}, "created_at": "2026-01-01T00:00:00Z" } } },
                    { "example": { "type": "close", "code": 4002, "reason": "protocol mismatch" } }
                ]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn document_covers_every_versioned_router_path() {
        let document = document();
        let documented = document["paths"]
            .as_object()
            .expect("OpenAPI paths")
            .keys()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let source = include_str!("api.rs");
        let routed = source
            .split(".route(")
            .skip(1)
            .filter_map(|segment| {
                let start = segment.find('"')? + 1;
                let end = segment[start..].find('"')? + start;
                segment[start..end].strip_prefix("/api/v1")
            })
            .collect::<HashSet<_>>();

        assert_eq!(documented, routed);
        assert!(documented.len() >= 50);
        assert_eq!(document["openapi"], "3.1.0");
        assert!(document["components"]["schemas"]["WebSocketServerFrame"].is_object());
    }
}
