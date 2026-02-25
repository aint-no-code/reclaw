use subtle::ConstantTimeEq;

use crate::{
    application::config::AuthMode,
    protocol::{ERROR_UNAVAILABLE, ErrorShape},
};

#[derive(Debug, Clone, Copy)]
pub enum AuthFailureReason {
    MissingCredentials,
    InvalidCredentials,
}

pub fn authorize(
    mode: &AuthMode,
    auth: Option<&crate::protocol::ConnectAuth>,
) -> Result<(), AuthFailureReason> {
    match mode {
        AuthMode::None => Ok(()),
        AuthMode::Token(expected) => {
            let provided = auth.and_then(|value| value.token.as_deref());
            verify_secret(provided, expected)
        }
        AuthMode::Password(expected) => {
            let provided = auth.and_then(|value| value.password.as_deref());
            verify_secret(provided, expected)
        }
    }
}

fn verify_secret(provided: Option<&str>, expected: &str) -> Result<(), AuthFailureReason> {
    let Some(provided) = provided.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(AuthFailureReason::MissingCredentials);
    };

    if provided.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(AuthFailureReason::InvalidCredentials)
    }
}

#[must_use]
pub fn auth_failure_error(reason: AuthFailureReason) -> ErrorShape {
    match reason {
        AuthFailureReason::MissingCredentials => {
            ErrorShape::new(ERROR_UNAVAILABLE, "unauthorized: missing credentials")
        }
        AuthFailureReason::InvalidCredentials => {
            ErrorShape::new(ERROR_UNAVAILABLE, "unauthorized: invalid credentials")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthFailureReason, authorize};
    use crate::{application::config::AuthMode, protocol::ConnectAuth};

    #[test]
    fn authorize_accepts_matching_secret() {
        let mode = AuthMode::Token("abc".to_owned());
        let auth = ConnectAuth {
            token: Some("abc".to_owned()),
            device_token: None,
            password: None,
        };

        assert!(authorize(&mode, Some(&auth)).is_ok());
    }

    #[test]
    fn authorize_rejects_invalid_secret() {
        let mode = AuthMode::Password("abc".to_owned());
        let auth = ConnectAuth {
            token: None,
            device_token: None,
            password: Some("zzz".to_owned()),
        };

        let result = authorize(&mode, Some(&auth));
        assert!(matches!(result, Err(AuthFailureReason::InvalidCredentials)));
    }
}
