use axum::http::HeaderMap;

pub fn verify_token(headers: &HeaderMap, expected: &str) -> bool {
    if let Some(auth) = headers.get("authorization") {
        if let Ok(value) = auth.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return token == expected;
            }
        }
    }

    if let Some(token) = headers.get("x-asr-token") {
        if let Ok(value) = token.to_str() {
            return value == expected;
        }
    }

    false
}
