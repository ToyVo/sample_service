//! xitdb backend round trips: temp-file databases, never `./dev.db`.

#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]

use api::auth::{AuthConnection, PasskeyData, UserData};
use api::models::{SampleData, UUID};
use api::store::{SampleConnection, StoreError};
use api::xitdb_backend::XitdbConnection;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn backend() -> XitdbConnection {
    // Unique per test even when tests run in parallel threads.
    let n = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("xitdb-backend-test-{}-{n}.db", std::process::id()));
    XitdbConnection::open(&path).expect("temp xitdb opens")
}

fn user(username: &str) -> UserData {
    UserData {
        id: UUID::random(),
        username: username.to_string(),
        password_hash: "$argon2id$fake".to_string(),
        created_at: chrono::Utc::now().naive_utc(),
    }
}

fn sample_at(name: &str, micros: i64) -> SampleData {
    SampleData {
        id: UUID::random(),
        name: name.to_string(),
        description: None,
        status: "ACTIVE".to_string(),
        tags: None,
        image_uri: None,
        created_at: chrono::DateTime::from_timestamp_micros(micros)
            .expect("valid micros")
            .naive_utc(),
        updated_at: None,
    }
}

#[test]
fn fresh_file_behaves_like_migrated() {
    let mut db = backend();
    db.run_migrations().expect("xitdb needs no migrations");
    assert!(db.fetch_samples(0, 10).expect("empty").is_empty());
    assert!(matches!(
        db.fetch_sample(UUID::random()),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn samples_round_trip_and_miss() {
    let mut db = backend();
    assert_eq!(db.backend_name(), "xitdb");
    let row = sample_at("one", 1_700_000_000_000_000);
    db.insert_sample(row.clone()).expect("insert");
    assert_eq!(db.fetch_sample(row.id).expect("fetch"), row);
    assert!(matches!(
        db.fetch_sample(UUID::random()),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn duplicate_username_is_conflict() {
    let mut db = backend();
    db.insert_user(user("alice")).expect("first insert");
    assert!(matches!(
        db.insert_user(user("alice")),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        db.fetch_user_by_username("alice").expect("fetch").username,
        "alice"
    );
}

#[test]
fn pagination_follows_created_at_order() {
    let mut db = backend();
    for (name, micros) in [
        ("third", 3_000_000_000_000_000),
        ("first", 1_000_000_000_000_000),
        ("second", 2_000_000_000_000_000),
    ] {
        db.insert_sample(sample_at(name, micros)).expect("insert");
    }
    let page = db.fetch_samples(1, 1).expect("page");
    assert_eq!(page.len(), 1);
    assert_eq!(page.first().expect("row").name, "second");
    let all = db.fetch_samples(0, 10).expect("all");
    assert_eq!(all.len(), 3);
    assert_eq!(all.last().expect("row").name, "third");
}

#[test]
fn passkeys_list_oldest_first() {
    let mut db = backend();
    let alice = user("alice");
    db.insert_user(alice.clone()).expect("user");
    let mut ids = Vec::new();
    for byte in [1u8, 2u8] {
        let key = PasskeyData {
            id: UUID::random(),
            user_id: alice.id,
            credential_id: vec![byte],
            passkey: "{}".to_string(),
            created_at: chrono::Utc::now().naive_utc(),
            last_used_at: None,
        };
        ids.push(key.id);
        db.insert_passkey(key).expect("insert");
    }
    let listed = db.fetch_passkeys(alice.id).expect("list");
    let listed_ids: Vec<UUID> = listed.iter().map(|key| key.id).collect();
    assert_eq!(listed_ids, ids);

    let duplicate = PasskeyData {
        id: UUID::random(),
        user_id: alice.id,
        credential_id: vec![1u8],
        passkey: "{}".to_string(),
        created_at: chrono::Utc::now().naive_utc(),
        last_used_at: None,
    };
    assert!(matches!(
        db.insert_passkey(duplicate),
        Err(StoreError::Conflict(_))
    ));
}
