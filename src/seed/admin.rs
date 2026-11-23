use crate::{auth::hash_password, db::Db, error::AppResult};

pub async fn ensure_admin(db: &Db, username: &str, password: &str) -> AppResult<bool> {
    if db.user_count().await? > 0 {
        return Ok(false);
    }
    let hash = hash_password(password)?;
    db.create_user(username, &hash, "系统管理员", "admin", "")
        .await?;
    Ok(true)
}
