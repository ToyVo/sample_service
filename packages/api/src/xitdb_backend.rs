//! xitdb storage backend (opt-in `xitdb-backend` feature).
//!
//! One top-level [`HashMap`] holds a fixed key per table; rows are nested
//! maps. Secondary indexes use the same shapes as the xitdb README: a
//! [`SortedMap`] for creation-time ordering and username lookup, and a
//! per-user [`LinkedArrayList`] of passkey ids for oldest-first listing.
//! Datetimes are stored as UTC micros (`Int`); absent keys read as `None`.
//! Uniqueness is check-then-insert, race-free under the single-`Mutex`
//! discipline documented in [`crate::state`].
//!
//! Row helpers return xitdb errors; the [`SampleConnection`] and
//! [`AuthConnection`] method bodies convert once at the boundary, and
//! business-level failures ([`StoreError::NotFound`],
//! [`StoreError::Conflict`]) are decided in the methods themselves.

use std::path::Path;

use chrono::{DateTime, NaiveDateTime};
use xitdb::core::BufferedFileCore;
use xitdb::cursor::{ReadCursor, ReadWrite, Writeable};

/// Shorthand for cursors from this backend's database.
type DBCursor<'a> = ReadCursor<'a, BufferedFileCore, Sha1Hasher>;
use xitdb::database::{Database, InitOpts};
use xitdb::error::Error as XitdbError;
use xitdb::hasher::{Sha1Hasher, bytes_to_id};
use xitdb::hashmap::HashMap;
use xitdb::linked_arraylist::LinkedArrayList;
use xitdb::sorted_map::SortedMap;

use crate::auth::AuthConnection;
use crate::auth::store::{PasskeyData, UserData};
use crate::models::common::{SampleData, UUID};
use crate::store::{SampleConnection, StoreError};

/// Shorthand for write-mode maps.
type Map<'a> = HashMap<'a, BufferedFileCore, Sha1Hasher, ReadWrite>;

/// xitdb-backed storage. Open with [`XitdbConnection::open`]; everything
/// else goes through the [`SampleConnection`] and [`AuthConnection`] traits
/// so handlers never branch on the backend.
pub struct XitdbConnection {
    db: Database<BufferedFileCore, Sha1Hasher>,
}

impl XitdbConnection {
    /// Open (creating) the database file at `path`.
    ///
    /// # Errors
    ///
    /// Returns the filesystem or xitdb error when the file cannot be opened.
    pub fn open(path: &Path) -> Result<Self, XitdbError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(XitdbError::Io)?;
        let db = Database::open(
            BufferedFileCore::new(file, true)?,
            Sha1Hasher,
            InitOpts {
                hash_id: Some(bytes_to_id(*b"sha1")),
                file_id: None,
                fsync: true,
            },
        )?;
        Ok(Self { db })
    }

    /// Fetch a table cursor, if the table exists yet. Read-only: never
    /// runs an init path, so repeated reads cannot disturb the file.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the root is not a map (corruption).
    fn table(&self, name: &str) -> Result<Option<DBCursor<'_>>, StoreError> {
        let root = HashMap::new(self.db.root_cursor()).map_err(StoreError::from)?;
        root.get(name).map_err(StoreError::from)
    }
}

impl From<XitdbError> for StoreError {
    fn from(err: XitdbError) -> Self {
        match err {
            XitdbError::KeyNotFound => Self::NotFound,
            _ => Self::Backend(err.to_string()),
        }
    }
}

/// Read a required string field.
fn str_field<M>(
    map: &HashMap<'_, BufferedFileCore, Sha1Hasher, M>,
    key: &str,
) -> Result<String, XitdbError> {
    let cursor = map.get(key)?.ok_or(XitdbError::KeyNotFound)?;
    let bytes = cursor.read_bytes_alloc()?;
    String::from_utf8(bytes).map_err(|_| XitdbError::Corrupted)
}

/// Read an optional string field (absent key means `None`).
fn opt_str_field<M>(
    map: &HashMap<'_, BufferedFileCore, Sha1Hasher, M>,
    key: &str,
) -> Result<Option<String>, XitdbError> {
    match map.get(key)? {
        None => Ok(None),
        Some(cursor) => {
            let bytes = cursor.read_bytes_alloc()?;
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| XitdbError::Corrupted)
        }
    }
}

/// Read a required timestamp field (UTC micros).
fn time_field<M>(
    map: &HashMap<'_, BufferedFileCore, Sha1Hasher, M>,
    key: &str,
) -> Result<NaiveDateTime, XitdbError> {
    micros_to_time(map.get(key)?.ok_or(XitdbError::KeyNotFound)?.read_int()?)
}

/// Read an optional timestamp field.
fn opt_time_field<M>(
    map: &HashMap<'_, BufferedFileCore, Sha1Hasher, M>,
    key: &str,
) -> Result<Option<NaiveDateTime>, XitdbError> {
    match map.get(key)? {
        None => Ok(None),
        Some(cursor) => micros_to_time(cursor.read_int()?).map(Some),
    }
}

/// Decode UTC micros, rejecting out-of-range values.
fn micros_to_time(micros: i64) -> Result<NaiveDateTime, XitdbError> {
    DateTime::from_timestamp_micros(micros)
        .map(|stamped| stamped.naive_utc())
        .ok_or(XitdbError::Corrupted)
}

/// Write a string field, removing it when `None` (absent key reads as `None`).
fn put_opt_str(map: &Map<'_>, key: &str, value: Option<&str>) -> Result<(), XitdbError> {
    value.map_or_else(
        || map.remove(key).map(|_| ()),
        |text| map.put(key, Writeable::Bytes(text.as_bytes().to_vec())),
    )
}

/// Encode an index key that sorts chronologically: flipping the sign bit
/// makes big-endian bytes order like the full `i64` range, and the id
/// suffix keeps keys distinct.
fn order_key(micros: i64, id: &str) -> Vec<u8> {
    let mut key = (micros ^ i64::MIN).to_be_bytes().to_vec();
    key.push(b'/');
    key.extend_from_slice(id.as_bytes());
    key
}

/// Parse a hyphenated UUID id string.
fn parse_id(id: &str) -> Result<UUID, XitdbError> {
    uuid::Uuid::parse_str(id)
        .map(UUID)
        .map_err(|_| XitdbError::Corrupted)
}

/// Read one sample row.
fn read_sample(entry: DBCursor<'_>, id: &str) -> Result<SampleData, XitdbError> {
    let row = HashMap::new(entry)?;
    Ok(SampleData {
        id: parse_id(id)?,
        name: str_field(&row, "name")?,
        description: opt_str_field(&row, "description")?,
        status: str_field(&row, "status")?,
        tags: opt_str_field(&row, "tags")?,
        image_uri: opt_str_field(&row, "image_uri")?,
        created_at: time_field(&row, "created_at")?,
        updated_at: opt_time_field(&row, "updated_at")?,
    })
}

/// Write one sample row (all fields; `None` removes the key).
fn write_sample(map: &Map<'_>, data: &SampleData) -> Result<(), XitdbError> {
    let id = data.id.0.to_string();
    let row = HashMap::new_write(&map.put_cursor(&id)?)?;
    row.put("id", Writeable::Bytes(id.into_bytes()))?;
    row.put("name", Writeable::Bytes(data.name.as_bytes().to_vec()))?;
    put_opt_str(&row, "description", data.description.as_deref())?;
    row.put("status", Writeable::Bytes(data.status.as_bytes().to_vec()))?;
    put_opt_str(&row, "tags", data.tags.as_deref())?;
    put_opt_str(&row, "image_uri", data.image_uri.as_deref())?;
    row.put(
        "created_at",
        Writeable::Int(data.created_at.and_utc().timestamp_micros()),
    )?;
    match data.updated_at {
        Some(stamp) => row.put(
            "updated_at",
            Writeable::Int(stamp.and_utc().timestamp_micros()),
        )?,
        None => {
            row.remove("updated_at")?;
        }
    }
    Ok(())
}

/// Read one user row.
fn read_user(entry: DBCursor<'_>, id: &str) -> Result<UserData, XitdbError> {
    let row = HashMap::new(entry)?;
    Ok(UserData {
        id: parse_id(id)?,
        username: str_field(&row, "username")?,
        password_hash: str_field(&row, "password_hash")?,
        created_at: time_field(&row, "created_at")?,
    })
}

/// Read one passkey row.
fn read_passkey(entry: DBCursor<'_>, id: &str) -> Result<PasskeyData, XitdbError> {
    let row = HashMap::new(entry)?;
    Ok(PasskeyData {
        id: parse_id(id)?,
        user_id: parse_id(&str_field(&row, "user_id")?)?,
        credential_id: row
            .get("credential_id")?
            .ok_or(XitdbError::KeyNotFound)?
            .read_bytes_alloc()?,
        passkey: str_field(&row, "passkey")?,
        created_at: time_field(&row, "created_at")?,
        last_used_at: opt_time_field(&row, "last_used_at")?,
    })
}

impl SampleConnection for XitdbConnection {
    fn backend_name(&self) -> &'static str {
        "xitdb"
    }

    fn run_migrations(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        // Collections materialize on first write and missing keys already
        // read as empty, so a fresh file behaves like a migrated database.
        Ok(())
    }

    fn insert_sample(&mut self, data: SampleData) -> Result<(), StoreError> {
        let duplicate = match self.table("samples")? {
            Some(tables) => HashMap::new(tables)?.get(&data.id.0.to_string())?.is_some(),
            None => false,
        };
        if duplicate {
            return Err(StoreError::Conflict("duplicate sample id".to_string()));
        }
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let samples = HashMap::new_write(&root.put_cursor("samples")?)?;
            write_sample(&samples, &data)?;
            let index = SortedMap::new_write(&root.put_cursor("samples_by_created")?)?;
            let id = data.id.0.to_string();
            index.put_by_bytes(
                &order_key(data.created_at.and_utc().timestamp_micros(), &id),
                Writeable::Bytes(id.into_bytes()),
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn fetch_sample(&mut self, id: UUID) -> Result<SampleData, StoreError> {
        let map = HashMap::new(self.table("samples")?.ok_or(StoreError::NotFound)?)?;
        let entry = map.get(&id.0.to_string())?.ok_or(StoreError::NotFound)?;
        Ok(read_sample(entry, &id.0.to_string())?)
    }

    fn fetch_samples(&mut self, offset: i64, limit: i64) -> Result<Vec<SampleData>, StoreError> {
        let entry = self.table("samples")?.ok_or(StoreError::NotFound)?;
        let samples = HashMap::new(entry)?;
        let index_entry = self
            .table("samples_by_created")?
            .ok_or(StoreError::NotFound)?;
        let index = SortedMap::new(index_entry)?;
        let mut rows = Vec::new();
        let mut walker = index.iter_from_index(offset)?;
        for _ in 0..limit {
            let Some(entry) = walker.next() else { break };
            let id = entry?.read_key_value_pair()?.value.read_bytes_alloc()?;
            let id = String::from_utf8(id)
                .map_err(|_| StoreError::Backend("invalid utf-8".to_string()))?;
            rows.push(read_sample(
                samples.get(&id)?.ok_or(StoreError::NotFound)?,
                &id,
            )?);
        }
        Ok(rows)
    }

    fn save_sample(&mut self, data: SampleData) -> Result<SampleData, StoreError> {
        if HashMap::new(self.table("samples")?.ok_or(StoreError::NotFound)?)?
            .get(&data.id.0.to_string())?
            .is_none()
        {
            return Err(StoreError::NotFound);
        }
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let samples = HashMap::new_write(&root.put_cursor("samples")?)?;
            // `created_at` never changes, so the index key stays valid.
            write_sample(&samples, &data)?;
            Ok(())
        })?;
        Ok(data)
    }

    fn delete_sample(&mut self, id: UUID) -> Result<(), StoreError> {
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let samples = HashMap::new_write(&root.put_cursor("samples")?)?;
            let id_string = id.0.to_string();
            if let Some(entry) = samples.get(&id_string)? {
                let row = HashMap::new(entry)?;
                let created = time_field(&row, "created_at")?;
                let index = SortedMap::new_write(&root.put_cursor("samples_by_created")?)?;
                index.remove_by_bytes(&order_key(
                    created.and_utc().timestamp_micros(),
                    &id_string,
                ))?;
                samples.remove(&id_string)?;
            }
            Ok(())
        })?;
        Ok(())
    }
}

impl AuthConnection for XitdbConnection {
    fn insert_user(&mut self, user: UserData) -> Result<(), StoreError> {
        let taken = match self.table("usernames")? {
            Some(names) => SortedMap::new(names)?.get(&user.username)?.is_some(),
            None => false,
        };
        if taken {
            return Err(StoreError::Conflict("username taken".to_string()));
        }
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let users = HashMap::new_write(&root.put_cursor("users")?)?;
            let names = SortedMap::new_write(&root.put_cursor("usernames")?)?;
            let id = user.id.0.to_string();
            let row = HashMap::new_write(&users.put_cursor(&id)?)?;
            row.put("id", Writeable::Bytes(id.as_bytes().to_vec()))?;
            row.put(
                "username",
                Writeable::Bytes(user.username.as_bytes().to_vec()),
            )?;
            names.put(&user.username, Writeable::Bytes(id.as_bytes().to_vec()))?;
            row.put(
                "password_hash",
                Writeable::Bytes(user.password_hash.as_bytes().to_vec()),
            )?;
            row.put(
                "created_at",
                Writeable::Int(user.created_at.and_utc().timestamp_micros()),
            )?;
            Ok(())
        })?;
        Ok(())
    }

    fn fetch_user(&mut self, id: UUID) -> Result<UserData, StoreError> {
        let map = HashMap::new(self.table("users")?.ok_or(StoreError::NotFound)?)?;
        let entry = map.get(&id.0.to_string())?.ok_or(StoreError::NotFound)?;
        Ok(read_user(entry, &id.0.to_string())?)
    }

    fn fetch_user_by_username(&mut self, username: &str) -> Result<UserData, StoreError> {
        // A dangling index entry reads as missing (fail closed): callers
        // map that to invalid credentials, never to another user.
        let map = SortedMap::new(self.table("usernames")?.ok_or(StoreError::NotFound)?)?;
        let entry = map.get(username)?.ok_or(StoreError::NotFound)?;
        let id = entry.read_bytes_alloc()?;
        let id =
            String::from_utf8(id).map_err(|_| StoreError::Backend("invalid utf-8".to_string()))?;
        let users = HashMap::new(self.table("users")?.ok_or(StoreError::NotFound)?)?;
        Ok(read_user(
            users.get(&id)?.ok_or(StoreError::NotFound)?,
            &id,
        )?)
    }

    fn insert_passkey(&mut self, passkey: PasskeyData) -> Result<(), StoreError> {
        let registered = match self.table("passkeys_by_credential")? {
            Some(index) => HashMap::new(index)?
                .get_by_bytes(&passkey.credential_id)?
                .is_some(),
            None => false,
        };
        if registered {
            return Err(StoreError::Conflict("credential id registered".to_string()));
        }
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let index = HashMap::new_write(&root.put_cursor("passkeys_by_credential")?)?;
            let keys = HashMap::new_write(&root.put_cursor("passkeys")?)?;
            let id = passkey.id.0.to_string();
            index.put_by_bytes(
                &passkey.credential_id,
                Writeable::Bytes(id.as_bytes().to_vec()),
            )?;
            let row = HashMap::new_write(&keys.put_cursor(&id)?)?;
            row.put("id", Writeable::Bytes(id.as_bytes().to_vec()))?;
            row.put(
                "user_id",
                Writeable::Bytes(passkey.user_id.0.to_string().into_bytes()),
            )?;
            row.put(
                "credential_id",
                Writeable::Bytes(passkey.credential_id.clone()),
            )?;
            row.put(
                "passkey",
                Writeable::Bytes(passkey.passkey.as_bytes().to_vec()),
            )?;
            row.put(
                "created_at",
                Writeable::Int(passkey.created_at.and_utc().timestamp_micros()),
            )?;
            match passkey.last_used_at {
                Some(stamp) => row.put(
                    "last_used_at",
                    Writeable::Int(stamp.and_utc().timestamp_micros()),
                )?,
                None => {
                    row.remove("last_used_at")?;
                }
            }
            let lists = HashMap::new_write(&root.put_cursor("passkey_lists")?)?;
            let user_list =
                LinkedArrayList::new_write(&lists.put_cursor(&passkey.user_id.0.to_string())?)?;
            user_list.append(Writeable::Bytes(id.as_bytes().to_vec()))?;
            Ok(())
        })?;
        Ok(())
    }

    fn fetch_passkeys(&mut self, user_id: UUID) -> Result<Vec<PasskeyData>, StoreError> {
        let entry = self.table("passkey_lists")?.ok_or(StoreError::NotFound)?;
        let Some(list) = HashMap::new(entry)?.get(&user_id.0.to_string())? else {
            return Ok(Vec::new());
        };
        let list = LinkedArrayList::new(list)?;
        let keys = self.table("passkeys")?.ok_or(StoreError::NotFound)?;
        let keys = HashMap::new(keys)?;
        let mut out = Vec::new();
        for entry in list.try_iter()? {
            let id = entry?.read_bytes_alloc()?;
            let id = String::from_utf8(id)
                .map_err(|_| StoreError::Backend("invalid utf-8".to_string()))?;
            out.push(read_passkey(
                keys.get(&id)?.ok_or(StoreError::NotFound)?,
                &id,
            )?);
        }
        Ok(out)
    }

    fn touch_passkey(
        &mut self,
        id: UUID,
        passkey: String,
        used_at: NaiveDateTime,
    ) -> Result<(), StoreError> {
        self.db.run_context(|w| {
            let root = HashMap::new_write(w)?;
            let keys = HashMap::new_write(&root.put_cursor("passkeys")?)?;
            let id_string = id.0.to_string();
            if keys.get(&id_string)?.is_none() {
                return Ok(());
            }
            let row = HashMap::new_write(&keys.put_cursor(&id_string)?)?;
            row.put("passkey", Writeable::Bytes(passkey.as_bytes().to_vec()))?;
            row.put(
                "last_used_at",
                Writeable::Int(used_at.and_utc().timestamp_micros()),
            )?;
            Ok(())
        })?;
        Ok(())
    }
}
