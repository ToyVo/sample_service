//! Backend-agnostic account and passkey storage.
//!
//! Mirrors [`crate::store`]: [`AuthConnection`] is implemented once per
//! backend by [`impl_auth_connection`], and is a supertrait of
//! [`crate::store::SampleConnection`] so the single boxed connection in
//! [`crate::AppState`] serves both samples and auth. Sessions are stateless
//! tokens (see [`super::token`]), so nothing here stores them.
//!
//! Rows are read and written as tuples rather than per-backend `Queryable`
//! structs, so the domain types below are the only row representation.

use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::models::UUID;
use crate::store::StoreError;

/// A user account. `username` is always stored lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserData {
    pub id: UUID,
    pub username: String,
    /// PHC-format argon2id hash.
    pub password_hash: String,
    pub created_at: NaiveDateTime,
}

/// A registered passkey. `passkey` is the JSON-serialized
/// `webauthn_rs::prelude::Passkey` (public key, counter, backup state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyData {
    pub id: UUID,
    pub user_id: UUID,
    pub credential_id: Vec<u8>,
    pub passkey: String,
    pub created_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
}

type UserRow = (UUID, String, String, NaiveDateTime);
type PasskeyRow = (
    UUID,
    UUID,
    Vec<u8>,
    String,
    NaiveDateTime,
    Option<NaiveDateTime>,
);

impl From<UserRow> for UserData {
    fn from((id, username, password_hash, created_at): UserRow) -> Self {
        Self {
            id,
            username,
            password_hash,
            created_at,
        }
    }
}

impl From<PasskeyRow> for PasskeyData {
    fn from((id, user_id, credential_id, passkey, created_at, last_used_at): PasskeyRow) -> Self {
        Self {
            id,
            user_id,
            credential_id,
            passkey,
            created_at,
            last_used_at,
        }
    }
}

/// Storage primitives for accounts and passkeys.
///
/// Object-safe like [`crate::store::SampleConnection`]; single-row lookups
/// return [`StoreError::NotFound`] when no row matches.
pub trait AuthConnection: Send {
    /// Insert a user.
    ///
    /// # Errors
    ///
    /// Returns a `UniqueViolation` database error when the username is taken.
    fn insert_user(&mut self, user: UserData) -> Result<(), StoreError>;

    /// Fetch a user by id.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] when no user matches.
    fn fetch_user(&mut self, id: UUID) -> Result<UserData, StoreError>;

    /// Fetch a user by (already lowercased) username.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] when no user matches.
    fn fetch_user_by_username(&mut self, username: &str) -> Result<UserData, StoreError>;

    /// Insert a passkey.
    ///
    /// # Errors
    ///
    /// Returns a `UniqueViolation` database error when the credential id is
    /// already registered.
    fn insert_passkey(&mut self, passkey: PasskeyData) -> Result<(), StoreError>;

    /// Fetch every passkey for a user, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the query fails.
    fn fetch_passkeys(&mut self, user_id: UUID) -> Result<Vec<PasskeyData>, StoreError>;

    /// Store a passkey's refreshed state (signature counter, backup flags)
    /// after a successful sign-in.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the update fails.
    fn touch_passkey(
        &mut self,
        id: UUID,
        passkey: String,
        used_at: NaiveDateTime,
    ) -> Result<(), StoreError>;
}

/// Generate an [`AuthConnection`] implementation for one backend's schema
/// module. Invoked once per backend, like `impl_sample_connection`.
macro_rules! impl_auth_connection {
    ($conn:ty, $schema:ident) => {
        impl AuthConnection for $conn {
            fn insert_user(&mut self, user: UserData) -> Result<(), StoreError> {
                use crate::schemas::$schema::users;
                diesel::insert_into(users::table)
                    .values((
                        users::id.eq(user.id),
                        users::username.eq(user.username),
                        users::password_hash.eq(user.password_hash),
                        users::created_at.eq(user.created_at),
                    ))
                    .execute(self)
                    .map(|_| ())
                    .map_err(StoreError::from)
            }

            fn fetch_user(&mut self, id: UUID) -> Result<UserData, StoreError> {
                use crate::schemas::$schema::users;
                users::table
                    .filter(users::id.eq(id))
                    .select((
                        users::id,
                        users::username,
                        users::password_hash,
                        users::created_at,
                    ))
                    .first::<UserRow>(self)
                    .map(UserData::from)
                    .map_err(StoreError::from)
            }

            fn fetch_user_by_username(&mut self, username: &str) -> Result<UserData, StoreError> {
                use crate::schemas::$schema::users;
                users::table
                    .filter(users::username.eq(username))
                    .select((
                        users::id,
                        users::username,
                        users::password_hash,
                        users::created_at,
                    ))
                    .first::<UserRow>(self)
                    .map(UserData::from)
                    .map_err(StoreError::from)
            }

            fn insert_passkey(&mut self, passkey: PasskeyData) -> Result<(), StoreError> {
                use crate::schemas::$schema::passkeys;
                diesel::insert_into(passkeys::table)
                    .values((
                        passkeys::id.eq(passkey.id),
                        passkeys::user_id.eq(passkey.user_id),
                        passkeys::credential_id.eq(passkey.credential_id),
                        passkeys::passkey.eq(passkey.passkey),
                        passkeys::created_at.eq(passkey.created_at),
                        passkeys::last_used_at.eq(passkey.last_used_at),
                    ))
                    .execute(self)
                    .map(|_| ())
                    .map_err(StoreError::from)
            }

            fn fetch_passkeys(&mut self, user_id: UUID) -> Result<Vec<PasskeyData>, StoreError> {
                use crate::schemas::$schema::passkeys;
                let rows: Vec<PasskeyRow> = passkeys::table
                    .filter(passkeys::user_id.eq(user_id))
                    .order(passkeys::created_at.asc())
                    .select((
                        passkeys::id,
                        passkeys::user_id,
                        passkeys::credential_id,
                        passkeys::passkey,
                        passkeys::created_at,
                        passkeys::last_used_at,
                    ))
                    .load(self)?;
                Ok(rows.into_iter().map(PasskeyData::from).collect())
            }

            fn touch_passkey(
                &mut self,
                id: UUID,
                passkey: String,
                used_at: NaiveDateTime,
            ) -> Result<(), StoreError> {
                use crate::schemas::$schema::passkeys;
                diesel::update(passkeys::table.filter(passkeys::id.eq(id)))
                    .set((
                        passkeys::passkey.eq(passkey),
                        passkeys::last_used_at.eq(Some(used_at)),
                    ))
                    .execute(self)
                    .map(|_| ())
                    .map_err(StoreError::from)
            }
        }
    };
}

impl_auth_connection!(diesel::sqlite::SqliteConnection, sqlite);
impl_auth_connection!(diesel::pg::PgConnection, postgresql);

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::StoreError;
    use diesel::sqlite::SqliteConnection;

    fn migrated_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite connects");
        crate::models::sqlite::run_sqlite_migrations(&mut conn).expect("migrations apply");
        conn
    }

    fn now() -> NaiveDateTime {
        chrono::Utc::now().naive_utc()
    }

    fn user(username: &str) -> UserData {
        UserData {
            id: UUID::random(),
            username: username.to_string(),
            password_hash: "$argon2id$fake".to_string(),
            created_at: now(),
        }
    }

    #[test]
    fn users_round_trip_and_reject_duplicate_usernames() {
        let mut conn = migrated_connection();
        let alice = user("alice");
        conn.insert_user(alice.clone()).expect("insert");
        assert_eq!(conn.fetch_user(alice.id).expect("by id"), alice);
        assert_eq!(
            conn.fetch_user_by_username("alice").expect("by name"),
            alice
        );
        assert!(matches!(
            conn.fetch_user(UUID::random()),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            conn.fetch_user_by_username("bob"),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            conn.insert_user(user("alice")),
            Err(StoreError::Conflict(_))
        ));
    }

    #[test]
    fn passkeys_list_per_user_and_update_in_place() {
        let mut conn = migrated_connection();
        let alice = user("alice");
        conn.insert_user(alice.clone()).expect("insert user");
        let key = PasskeyData {
            id: UUID::random(),
            user_id: alice.id,
            credential_id: vec![9, 9, 9],
            passkey: "{}".to_string(),
            created_at: now(),
            last_used_at: None,
        };
        conn.insert_passkey(key.clone()).expect("insert passkey");
        assert_eq!(
            conn.fetch_passkeys(alice.id).expect("list"),
            vec![key.clone()]
        );
        assert_eq!(
            conn.fetch_passkeys(UUID::random()).expect("list"),
            Vec::new()
        );

        // The same credential cannot be registered twice.
        let duplicate = PasskeyData {
            id: UUID::random(),
            ..key.clone()
        };
        assert!(conn.insert_passkey(duplicate).is_err());

        let used_at = now();
        conn.touch_passkey(key.id, r#"{"updated":true}"#.to_string(), used_at)
            .expect("touch");
        let refreshed = conn.fetch_passkeys(alice.id).expect("list");
        let first = refreshed.first().expect("one");
        assert_eq!(first.passkey, r#"{"updated":true}"#);
        assert_eq!(first.last_used_at, Some(used_at));
    }
}
