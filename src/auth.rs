use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
};
use jsonwebtoken::{encode, Algorithm, Header};
use serde::{Deserialize, Serialize};

use crate::{auth_keys::JwtKeys, error::AppError, state::AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub role: String,
    #[serde(default)]
    pub org_name: String,
    pub exp: usize,
    pub iat: usize,
    pub jti: String,
    pub iss: String,
    pub aud: String,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub role: String,
    pub org_name: String,
    pub jti: String,
}

impl AuthUser {
    pub fn can_write(&self) -> bool {
        matches!(self.role.as_str(), "admin" | "operator")
    }

    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    /// Empty org means "all organizations" for administrators; non-admin
    /// accounts without an org see no vehicle rows.
    pub fn org_filter(&self) -> &str {
        if self.is_admin() {
            "*"
        } else {
            &self.org_name
        }
    }

    pub fn can_access_org(&self, org_name: &str) -> bool {
        self.is_admin() || self.org_name == org_name
    }

    pub fn require_write(&self) -> Result<(), AppError> {
        if self.can_write() {
            Ok(())
        } else {
            Err(AppError::forbidden("当前账号为只读角色，无权执行该操作"))
        }
    }

    pub fn require_admin(&self) -> Result<(), AppError> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(AppError::forbidden("该操作仅限管理员"))
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::unauthorized("缺少登录凭证"))?;

        let claims = decode_token(&state.jwt, token)?;
        if state.is_access_revoked(&claims.jti).await? {
            return Err(AppError::unauthorized("登录凭证已注销"));
        }
        Ok(AuthUser {
            id: claims.sub,
            username: claims.username,
            role: claims.role,
            org_name: claims.org_name,
            jti: claims.jti,
        })
    }
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| {
            tracing::error!(%error, "password hashing failed");
            AppError::internal("密码处理失败")
        })
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub fn create_token(
    jwt: &JwtKeys,
    user_id: &str,
    username: &str,
    role: &str,
    org_name: &str,
) -> Result<String, AppError> {
    let now = chrono::Utc::now();
    let ttl = chrono::Duration::minutes(jwt.access_ttl_minutes.max(1));
    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        role: role.to_string(),
        org_name: org_name.to_string(),
        iat: now.timestamp() as usize,
        exp: (now + ttl).timestamp() as usize,
        jti: uuid::Uuid::new_v4().to_string(),
        iss: jwt.issuer.clone(),
        aud: jwt.audience.clone(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(jwt.kid.clone());
    encode(&header, &claims, &jwt.encoding).map_err(|error| {
        tracing::error!(%error, "jwt encoding failed");
        AppError::internal("生成登录凭证失败")
    })
}

pub fn decode_token(jwt: &JwtKeys, token: &str) -> Result<Claims, AppError> {
    let key = jwt.decoding_for(token)?;
    let data = jsonwebtoken::decode::<Claims>(token, key, &jwt.validation()).map_err(|error| {
        let message = match error.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => "登录已过期，请重新登录",
            _ => "登录凭证无效",
        };
        AppError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    })?;
    Ok(data.claims)
}
