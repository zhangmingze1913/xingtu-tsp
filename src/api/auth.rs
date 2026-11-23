use axum::{extract::State, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::{
    auth::{create_token, verify_password, AuthUser},
    error::{AppError, AppResult},
    models::{LoginRequest, LoginResponse, LogoutRequest, RefreshRequest, RefreshResponse, User},
    state::AppState,
};

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Json<LoginResponse>> {
    let username = payload.username.trim();
    if username.is_empty() || payload.password.is_empty() {
        return Err(AppError::bad_request("请输入用户名和密码"));
    }
    let since = crate::util::iso_from_now_minutes(-15);
    if state.db.recent_login_failures(username, &since).await? >= 5 {
        return Err(AppError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            "登录失败次数过多，请 15 分钟后再试",
        ));
    }
    let user = state.db.find_user_by_username(username).await?;
    let Some((user, password_hash)) = user else {
        return Err(AppError::unauthorized("用户名或密码不正确"));
    };
    let password = payload.password.clone();
    let hash = password_hash.clone();
    let verified = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(|_| AppError::internal("密码校验任务失败"))?;
    if !verified {
        let _ = state
            .db
            .audit(
                Some(&user.id),
                Some(&user.username),
                "auth.login_failed",
                &user.id,
                "密码校验失败",
            )
            .await;
        return Err(AppError::unauthorized("用户名或密码不正确"));
    }
    let (access_token, refresh_token, expires_in) = issue_session(&state, &user).await?;
    state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "auth.login",
            &user.id,
            "登录成功",
        )
        .await?;
    Ok(Json(LoginResponse {
        token: access_token.clone(),
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
        user,
    }))
}

pub async fn refresh(
    State(state): State<AppState>,
    Json(payload): Json<RefreshRequest>,
) -> AppResult<Json<RefreshResponse>> {
    let token_hash = hash_refresh_token(&payload.refresh_token);
    let Some(record) = state.db.find_active_refresh(&token_hash).await? else {
        return Err(AppError::unauthorized("刷新凭证无效或已过期"));
    };
    let Some(user) = state.db.find_user_by_id(&record.user_id).await? else {
        return Err(AppError::unauthorized("账号不存在或已禁用"));
    };
    state.db.revoke_refresh(&record.id).await?;
    let (access_token, refresh_token, expires_in) = issue_session(&state, &user).await?;
    let _ = state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "auth.refresh",
            &user.id,
            "刷新访问凭证",
        )
        .await;
    Ok(Json(RefreshResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

pub async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<LogoutRequest>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .revoke_access(
            &user.jti,
            std::time::Duration::from_secs(
                (state.config.jwt_access_ttl_minutes.max(1) * 60) as u64,
            ),
        )
        .await?;
    if let Some(refresh_token) = payload.refresh_token.as_deref() {
        state
            .db
            .revoke_refresh_by_hash(&hash_refresh_token(refresh_token))
            .await?;
    }
    let _ = state
        .db
        .audit(
            Some(&user.id),
            Some(&user.username),
            "auth.logout",
            &user.id,
            "注销登录",
        )
        .await;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn jwks(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.jwt.jwks())
}

pub async fn me(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<User>> {
    let profile = state.db.find_user_by_id(&user.id).await?;
    match profile {
        Some(profile) => Ok(Json(profile)),
        None => Err(AppError::unauthorized("账号不存在或已被禁用")),
    }
}

async fn issue_session(state: &AppState, user: &User) -> AppResult<(String, String, i64)> {
    let access_token = create_token(
        &state.jwt,
        &user.id,
        &user.username,
        &user.role,
        &user.org_name,
    )?;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let refresh_token = URL_SAFE_NO_PAD.encode(bytes);
    let token_hash = hash_refresh_token(&refresh_token);
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::hours(state.config.jwt_refresh_ttl_hours.max(1)))
    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    state
        .db
        .store_refresh_token(
            &uuid::Uuid::new_v4().to_string(),
            &user.id,
            &token_hash,
            &expires_at,
        )
        .await?;
    Ok((
        access_token,
        refresh_token,
        state.config.jwt_access_ttl_minutes.max(1) * 60,
    ))
}

fn hash_refresh_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
