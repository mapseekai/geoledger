//! Closed JSON shapes for the current authentication protocol.
//! Validate before password work or storage admission; keep the caller's original
//! object so omitted expiry and an explicit null remain distinct during updates.
use crate::{Error, Result};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::Value;

fn optional<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct LoginPassword {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct LoginApiKey {
    api_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct ListUsers {
    #[serde(default, deserialize_with = "optional")]
    after: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    limit: Option<i64>,
    #[serde(default, deserialize_with = "optional")]
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct CreateUser {
    current_password: String,
    username: String,
    password: String,
    #[serde(default, deserialize_with = "optional")]
    display_name: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    is_admin: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct UpdateUser {
    current_password: String,
    user_id: String,
    #[serde(default, deserialize_with = "optional")]
    username: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    display_name: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    status: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    is_admin: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct ChangePassword {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct ResetPassword {
    current_password: String,
    user_id: String,
    new_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct ListApiKeys {
    #[serde(default, deserialize_with = "optional")]
    user_id: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    after: Option<String>,
    #[serde(default, deserialize_with = "optional")]
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct CreateApiKey {
    current_password: String,
    name: String,
    #[serde(default, deserialize_with = "optional")]
    user_id: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct UpdateApiKey {
    current_password: String,
    key_id: String,
    #[serde(default, deserialize_with = "optional")]
    name: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "shape-only validation retains the original JSON and its field presence"
)]
struct ApiKeyMutation {
    current_password: String,
    key_id: String,
}

fn shape<T: DeserializeOwned>(input: &Value) -> Result<()> {
    T::deserialize(input)
        .map(|_| ())
        .map_err(Error::invalid_json)
}

pub(super) fn validate(operation: &str, input: &Value) -> Result<()> {
    if !input.is_object() {
        return Err(Error::new(400, "authentication requires a JSON object"));
    }
    match operation {
        "login_password" => shape::<LoginPassword>(input),
        "login_api_key" => shape::<LoginApiKey>(input),
        "me" | "logout" => shape::<EmptyInput>(input),
        "list_users" => shape::<ListUsers>(input),
        "create_user" => shape::<CreateUser>(input),
        "update_user" => shape::<UpdateUser>(input),
        "change_password" => shape::<ChangePassword>(input),
        "reset_password" => shape::<ResetPassword>(input),
        "list_api_keys" => shape::<ListApiKeys>(input),
        "create_api_key" => shape::<CreateApiKey>(input),
        "update_api_key" => shape::<UpdateApiKey>(input),
        "revoke_api_key" | "delete_api_key" => shape::<ApiKeyMutation>(input),
        _ => Err(Error::new(400, "unknown authentication operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_authentication_operation_rejects_unknown_and_missing_fields()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                "login_password",
                json!({"username":"test","password":"test"}),
            ),
            ("login_api_key", json!({"api_key":"test"})),
            ("me", json!({})),
            ("logout", json!({})),
            ("list_users", json!({})),
            (
                "create_user",
                json!({"current_password":"test","username":"test","password":"test"}),
            ),
            (
                "update_user",
                json!({"current_password":"test","user_id":"test"}),
            ),
            (
                "change_password",
                json!({"current_password":"test","new_password":"test"}),
            ),
            (
                "reset_password",
                json!({"current_password":"test","user_id":"test","new_password":"test"}),
            ),
            ("list_api_keys", json!({})),
            (
                "create_api_key",
                json!({"current_password":"test","name":"test"}),
            ),
            (
                "update_api_key",
                json!({"current_password":"test","key_id":"test"}),
            ),
            (
                "revoke_api_key",
                json!({"current_password":"test","key_id":"test"}),
            ),
            (
                "delete_api_key",
                json!({"current_password":"test","key_id":"test"}),
            ),
        ];
        for (operation, input) in cases {
            assert!(validate(operation, &input).is_ok(), "{operation}");
            let mut unknown = input.clone();
            unknown["unrecognized"] = json!(true);
            assert_eq!(
                validate(operation, &unknown).err().map(|e| e.status),
                Some(400),
                "{operation}"
            );
            for key in input.as_object().ok_or("test object")?.keys() {
                let mut missing = input.clone();
                missing.as_object_mut().ok_or("test object")?.remove(key);
                assert!(validate(operation, &missing).is_err(), "{operation}: {key}");
                let mut null = input.clone();
                null[key] = Value::Null;
                assert!(validate(operation, &null).is_err(), "{operation}: {key}");
            }
            for invalid in [Value::Null, json!([]), json!("input")] {
                assert!(validate(operation, &invalid).is_err(), "{operation}");
            }
        }
        Ok(())
    }

    #[test]
    fn expiry_presence_and_other_optional_types_keep_the_current_contract() {
        for expires_at in [None, Some(Value::Null), Some(json!("2099-01-01T00:00:00Z"))] {
            let mut input = json!({"current_password":"test", "key_id":"key"});
            if let Some(value) = expires_at {
                input["expires_at"] = value;
            }
            assert!(validate("update_api_key", &input).is_ok());
        }
        for input in [
            json!({"current_password":"test", "key_id":"key", "name":null}),
            json!({"current_password":"test", "key_id":"key", "expires_at":7}),
            json!({"current_password":"test", "user_id":"user", "is_admin":"false"}),
        ] {
            let operation = if input.get("user_id").is_some() {
                "update_user"
            } else {
                "update_api_key"
            };
            assert!(validate(operation, &input).is_err());
        }
        assert!(validate("me", &json!({"current_password":"test"})).is_err());
        assert!(validate("list_users", &json!({"limit":null})).is_err());
        assert!(
            validate(
                "list_users",
                &json!({"limit":50,"after":"","status":"active"})
            )
            .is_ok()
        );
    }
}
