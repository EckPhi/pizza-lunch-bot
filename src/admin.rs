use crate::{Result, access::Change, weekly::Weekly};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use std::{
    env,
    sync::{Arc, Mutex},
};

type Signature = Hmac<Sha256>;

pub struct Key([u8; 32]);

impl Key {
    pub fn from_env() -> Result<Option<Self>> {
        let Some(secret) = env::var_os("PIZZA_ADMIN_TOKEN") else {
            return Ok(None);
        };
        let secret = secret
            .into_string()
            .map_err(|_| "Invalid PIZZA_ADMIN_TOKEN")?;
        if secret.len() < 32 || !secret.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err("Set PIZZA_ADMIN_TOKEN to at least 32 visible ASCII characters.".into());
        }
        Ok(Some(Self(
            Self::signature(&secret).finalize().into_bytes().into(),
        )))
    }

    fn signature(secret: &str) -> Signature {
        let mut mac =
            Signature::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(b"pizza-lunch-admin");
        mac
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .is_some_and(|secret| Self::signature(secret).verify_slice(&self.0).is_ok())
    }
}

struct Admin {
    key: Key,
    weekly: Arc<Mutex<Weekly>>,
}

pub fn router(key: Key, weekly: Arc<Mutex<Weekly>>) -> Router {
    Router::new()
        .route("/", get(|| async {
            ([
                (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
                (header::CACHE_CONTROL, "no-store"),
                (header::REFERRER_POLICY, "no-referrer"),
            ], Html(include_str!("admin/index.html")))
        }))
        .route("/app.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript")], include_str!("admin/app.js"))
        }))
        .route("/app.css", get(|| async {
            ([(header::CONTENT_TYPE, "text/css")], include_str!("admin/app.css"))
        }))
        .route("/api", get(read).post(update))
        .with_state(Arc::new(Admin { key, weekly }))
}

async fn read(State(admin): State<Arc<Admin>>, headers: HeaderMap) -> Response {
    directory(admin, headers, None).await
}

async fn update(
    State(admin): State<Arc<Admin>>,
    headers: HeaderMap,
    Json(change): Json<Change>,
) -> Response {
    directory(admin, headers, Some(change)).await
}

async fn directory(admin: Arc<Admin>, headers: HeaderMap, change: Option<Change>) -> Response {
    let result = if admin.key.accepts(&headers) {
        tokio::task::spawn_blocking(move || {
            let mut weekly = admin
                .weekly
                .lock()
                .map_err(|_| "Weekly state lock failed")?;
            let access = weekly
                .access
                .as_mut()
                .ok_or("Access directory is disabled")?;
            if let Some(change) = change
                && !access.apply(change)?
            {
                return Ok((
                    StatusCode::NOT_FOUND,
                    json!({"error": "This entry was not found."}),
                ));
            }
            Ok((StatusCode::OK, serde_json::to_value(access.directory())?))
        })
        .await
    } else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error": "Enter a valid admin token."}),
        );
    };
    match result {
        Ok(Ok((status, body))) => reply(status, body),
        Ok(Err(error)) => {
            let error: Box<dyn std::error::Error + Send + Sync> = error;
            eprintln!("Access directory request failed: {error}");
            reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "Could not access the directory."}),
            )
        }
        Err(_) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "Could not access the directory."}),
        ),
    }
}

fn reply(status: StatusCode, body: serde_json::Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], (status, Json(body))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{Kind, Rule};
    use std::{fs, path::PathBuf};

    #[test]
    fn access_changes_require_the_admin_key_and_survive_restart() -> Result<()> {
        let path = env::temp_dir().join(format!("pizza-admin-test-{}.json", std::process::id()));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(self.0.with_extension("access.json"));
            }
        }
        let _cleanup = Cleanup(path.clone());
        let mut weekly = Weekly::load(path.clone(), chrono_tz::Europe::Vienna)?;
        weekly.configure_access(true)?;
        let user = json!({"id":42,"first_name":"Ada"});
        let group = Some((-123, "Pizza Club"));
        assert!(weekly.authorize(group, &user)?);
        let token = "sample-test-token-longer-than-32-characters";
        let admin = Arc::new(Admin {
            key: Key(Key::signature(token).finalize().into_bytes().into()),
            weekly: Arc::new(Mutex::new(weekly)),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let change = || Change::SetRule {
            kind: Kind::Group,
            id: "-123".into(),
            rule: Rule::Block,
        };
        let denied = runtime.block_on(directory(
            Arc::clone(&admin),
            HeaderMap::new(),
            Some(change()),
        ));
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(admin.weekly.lock().unwrap().authorize(group, &user)?);

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse()?);
        let response = runtime.block_on(directory(
            Arc::clone(&admin),
            headers.clone(),
            Some(change()),
        ));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!admin.weekly.lock().unwrap().authorize(group, &user)?);

        for (change, allowed) in [
            (
                Change::SetAllowlist {
                    kind: Kind::User,
                    enabled: true,
                },
                false,
            ),
            (
                Change::SetRule {
                    kind: Kind::User,
                    id: "42".into(),
                    rule: Rule::Allow,
                },
                true,
            ),
            (
                Change::SetRule {
                    kind: Kind::User,
                    id: "42".into(),
                    rule: Rule::Block,
                },
                false,
            ),
        ] {
            let response =
                runtime.block_on(directory(Arc::clone(&admin), headers.clone(), Some(change)));
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                admin.weekly.lock().unwrap().authorize(None, &user)?,
                allowed
            );
        }
        let mut restarted = Weekly::load(path, chrono_tz::Europe::Vienna)?;
        restarted.configure_access(false)?;
        assert!(!restarted.authorize(group, &user)?);
        assert!(!restarted.authorize(None, &user)?);
        Ok(())
    }
}
